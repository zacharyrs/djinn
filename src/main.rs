#![warn(clippy::all)]
use std::env;
use std::ffi;
use std::fs;
use std::io::{self};
use std::process::Command;
use std::thread;
use std::time::Duration;

use clap::{
    app_from_crate, crate_authors, crate_description, crate_name, crate_version, AppSettings, Arg,
    SubCommand,
};
use figment::{
    providers::{Format, Serialized, Toml},
    Figment,
};
use nix::{mount, unistd};
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessExt, SystemExt};

static CONFIG_PATH: &str = "/etc/djinn.cfg";
static ENVARS: [&str; 3] = ["WSL_DISTRO_NAME", "WSL_INTEROP", "WSLENV"];

static VERBOSE: bool = false;
static PRESERVE_ENV: bool = false;
static SUFFIX: &str = "-wsl";
static UNSHARE: &str = "/usr/bin/unshare";
static DAEMONIZE: &str = "/usr/sbin/daemonize";

#[derive(Deserialize, Serialize)]
struct Config {
    verbose: bool,
    preserve_env: bool,
    suffix: String,
    unshare: String,
    daemonize: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            verbose: VERBOSE,
            preserve_env: PRESERVE_ENV,
            suffix: SUFFIX.to_string(),
            unshare: UNSHARE.to_string(),
            daemonize: DAEMONIZE.to_string(),
        }
    }
}

fn main() {
    let opts = app_from_crate!()
        .setting(AppSettings::InferSubcommands)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::DeriveDisplayOrder)
        .setting(AppSettings::DisableHelpSubcommand)
        .arg(Arg::with_name("verbose").short("v").help("Set verbosity"))
        .subcommands(vec![
            SubCommand::with_name("init").about("set up a bottle"),
            SubCommand::with_name("shell")
                .about("launch a shell inside bottle, sets up a bottle if necessary"),
            SubCommand::with_name("run")
                .about("run a command inside bottle, sets up a bottle if necessary")
                .setting(AppSettings::TrailingVarArg)
                .arg(
                    Arg::with_name("command")
                        .help("the command to run inside the bottle")
                        .multiple(true)
                        .required(true),
                ),
            SubCommand::with_name("cleanup").about("destroy the bottle bottle"),
        ])
        .get_matches();

    let verb: bool = opts.is_present("verbose");

    let config: Config = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::file(CONFIG_PATH))
        .extract()
        .expect("! djinn failed to create config");

    if unistd::geteuid() != unistd::Uid::from_raw(0) {
        println!("! djinn needs to be run as root - try the setuid bit");
        return;
    }

    // check we're in linux
    if !cfg!(unix) {
        println!("! djinn must be run within wsl 2");
        return;
    }

    // check for wsl1
    for line in fs::read_to_string("/proc/self/mounts").unwrap().lines() {
        let mnt: Vec<_> = line.split_whitespace().collect();
        if mnt[1] == "/" {
            if mnt[2] == "lxfs" || mnt[2] == "wslfs" {
                println!("! djinn must be run within wsl 2");
                return;
            } else {
                break;
            }
        }
    }

    // check for wsl2
    if !fs::metadata("/run/WSL").unwrap().is_dir() {
        println!("! djinn must be run within wsl 2");
        return;
    }

    let user_name: String = env::var("LOGNAME").expect("failed to get username");

    let user_id: unistd::Uid = unistd::getuid();
    let group_id: unistd::Gid = unistd::getgid();

    let mut systemd_pid: unistd::Pid =
        _get_systemd_pid().expect("failed to determine if systemd is running");

    let bottle_exists: bool;
    let bottle_inside: bool;

    if systemd_pid == unistd::Pid::from_raw(0) {
        bottle_exists = false;
        bottle_inside = false;
        if verb {
            println!("djinn: bottle doesn't exist")
        }
    } else if systemd_pid == unistd::Pid::from_raw(1) {
        bottle_exists = true;
        bottle_inside = true;
        if verb {
            println!("djinn: bottle exists, and we're in it")
        }
    } else {
        bottle_exists = true;
        bottle_inside = false;
        if verb {
            println!("djinn: bottle exists ({}), but we're outside", &systemd_pid)
        }
    }

    let mut envnames: String = String::new();
    let mut envars: Vec<ffi::CString> = vec![];

    if let Some(_subopts) = opts.subcommand_matches("init") {
        if bottle_exists {
            println!("djinn: no need to init - bottle exists");
            return;
        }
        _grab_root(verb);
        init(verb);
        _jump_user(verb, user_id, group_id);
        return;
    }

    if let Some(_subopts) = opts.subcommand_matches("shell") {
        if bottle_inside {
            println!("djinn: no need to make a shell - we're in the bottle");
            return;
        }
        _grab_root(verb);
        if !bottle_exists {
            systemd_pid = init(verb);
        }
        _get_env(&mut envars, &mut envnames);
        shell(verb, systemd_pid, &user_name, &envars, &envnames);
        _jump_user(verb, user_id, group_id);
        return;
    }

    if let Some(subopts) = opts.subcommand_matches("run") {
        let command_string: Vec<ffi::CString> = subopts
            .values_of("command")
            .unwrap()
            .map(|s| ffi::CString::new(s).unwrap())
            .collect();
        let command: Vec<&ffi::CStr> = command_string.iter().map(|s| s.as_c_str()).collect();
        if bottle_inside {
            println!("djinn: no need to enter bottle - we're in it already");
            _get_env(&mut envars, &mut envnames);

            let envars_obj: Vec<&ffi::CStr> = envars.iter().map(|s| s.as_c_str()).collect();
            unistd::execvpe(&command[0], &command, &envars_obj).expect("failed to launch shell");
            return;
        }
        _grab_root(verb);
        if !bottle_exists {
            systemd_pid = init(verb);
        }
        _get_env(&mut envars, &mut envnames);
        run(verb, systemd_pid, &user_name, &envars, command_string);
        _jump_user(verb, user_id, group_id);
        return;
    }

    if let Some(_subopts) = opts.subcommand_matches("cleanup") {
        if bottle_inside {
            println!("djinn: can't shutdown from inside bottle");
            return;
        }
        _grab_root(verb);
        if !bottle_exists {
            println!("djinn: no bottle exists to shutdown");
            return;
        }
        cleanup(verb, systemd_pid);
        _jump_user(verb, user_id, group_id);
        return;
    }
}

fn init(verb: bool) -> unistd::Pid {
    // init a bottle
    if verb {
        println!("djinn: beginning bottle init...");
    }

    let hostname = fs::read_to_string("/etc/hostname")
        .expect("failed to get hostname")
        .trim()
        .to_string();
    let new_hostname = format!("{}{}", hostname, SUFFIX);

    _set_hostname(verb, &new_hostname).expect("failed to set custom hostname");
    _set_hosts(verb, &hostname, &new_hostname).expect("failed to set custom hosts");

    _saveenv(verb).expect("failed to dump environment");

    let _systemd = Command::new("/usr/sbin/daemonize")
        .args(&[
            "/usr/bin/unshare",
            "-fp",
            "--propagation",
            "shared",
            "--mount-proc",
            "/lib/systemd/systemd",
        ])
        .output()
        .expect("failed to launch systemd");

    let mut systemd_pid: unistd::Pid = _get_systemd_pid().unwrap();

    while systemd_pid == unistd::Pid::from_raw(0) {
        thread::sleep(Duration::from_millis(500));
        systemd_pid = _get_systemd_pid().unwrap();
    }

    systemd_pid
}

fn cleanup(verb: bool, mut systemd_pid: unistd::Pid) {
    // destroy a bottle
    if verb {
        println!("djinn: beginning bottle destruction...");
    }

    let _exiter = Command::new("/usr/bin/nsenter")
        .args(&[
            "/usr/bin/nsenter",
            "-t",
            &format!("{}", systemd_pid),
            "-m",
            "-p",
            "/usr/sbin/systemctl",
            "poweroff",
        ])
        .output()
        .expect("failed to launch systemd");

    systemd_pid = _get_systemd_pid().unwrap();

    if systemd_pid != unistd::Pid::from_raw(0) {
        if verb {
            println!("djinn:  - waiting for bottle shutdown")
        }

        while systemd_pid != unistd::Pid::from_raw(0) {
            thread::sleep(Duration::from_millis(500));
            systemd_pid = _get_systemd_pid().unwrap();
        }
    }

    if verb {
        println!("djinn:  - bottle has shut down")
    }

    if mount::umount("/etc/hosts").is_err() {
        panic!(io::Error::last_os_error());
    }

    if mount::umount("/etc/hostname").is_err() {
        panic!(io::Error::last_os_error());
    }

    if verb {
        println!("djinn:  - bind mounts removed")
    }

    fs::remove_file("/run/djinn.env").expect("failed during tidyup");
    fs::remove_file("/run/djinn.hosts").expect("failed during tidyup");
    fs::remove_file("/run/djinn.hostname").expect("failed during tidyup");

    if verb {
        println!("djinn:  - files removed")
    }
}

fn shell(
    verb: bool,
    systemd_pid: unistd::Pid,
    user_name: &str,
    envars: &[ffi::CString],
    envnames: &str,
) {
    // launch a shell
    if verb {
        println!("djinn: let's make a shell");
    }

    let args: Vec<ffi::CString> = [
        "/usr/bin/nsenter",
        "-t",
        &format!("{}", systemd_pid),
        "-m",
        "-p",
        "/sbin/runuser",
        "-l",
        user_name,
        "-w",
        envnames,
    ]
    .iter()
    .map(|&s| ffi::CString::new(s).unwrap())
    .collect();

    let args_obj: Vec<&ffi::CStr> = args.iter().map(|s| s.as_c_str()).collect();
    let envars_obj: Vec<&ffi::CStr> = envars.iter().map(|s| s.as_c_str()).collect();

    unistd::execve(args_obj[0], &args_obj, &envars_obj).expect("failed to launch shell");
}

fn run(
    verb: bool,
    systemd_pid: unistd::Pid,
    user_name: &str,
    envars: &[ffi::CString],
    mut command: Vec<ffi::CString>,
) {
    // run something in the bottle
    if verb {
        println!("djinn: let's run something");
    }

    let mut args: Vec<ffi::CString> = [
        "/usr/bin/nsenter",
        "-t",
        &format!("{}", systemd_pid),
        &format!("--wd={}", env::current_dir().unwrap().display()),
        "-m",
        "-p",
        "/sbin/runuser",
        "-u",
        user_name,
        "--",
    ]
    .iter()
    .map(|&s| ffi::CString::new(s).unwrap())
    .collect();

    args.append(&mut command);

    let args_obj: Vec<&ffi::CStr> = args.iter().map(|s| s.as_c_str()).collect();

    let envars_obj: Vec<&ffi::CStr> = envars.iter().map(|s| s.as_c_str()).collect();

    unistd::execve(args_obj[0], &args_obj, &envars_obj).expect("failed to launch shell");
}

fn _set_hostname(verb: bool, hostname: &str) -> io::Result<()> {
    if verb {
        println!("djinn:  setting hostname via bind mount -> {}", hostname);
    }

    // save our custom hostname
    fs::write("/run/djinn.hostname", format!("{}\n", hostname))?;

    if mount::mount::<str, str, str, str>(
        Some("/run/djinn.hostname"),
        "/etc/hostname",
        None,
        mount::MsFlags::MS_BIND,
        None,
    )
    .is_err()
    {
        return Err(io::Error::last_os_error());
    }

    if verb {
        println!("djinn:  - successful");
    }
    Ok(())
}

fn _set_hosts(verb: bool, old: &str, new: &str) -> io::Result<()> {
    if verb {
        println!("djinn:  create patched hosts file: {} -> {}", old, new);
    }

    let hosts: String = fs::read_to_string("/etc/hosts")?;
    let mut out: String = String::new();

    for line in hosts.lines() {
        let mut bits = line.split_whitespace();
        if let Some(ip) = bits.next() {
            let patch = bits.map(|s| if s == old { &new } else { s });
            out.push_str(&format!(
                "{}\t{}\n",
                ip,
                patch.collect::<Vec<&str>>().join(" "),
            ))
        }
    }

    fs::write("/run/djinn.hosts", out)?;

    if verb {
        println!("djinn:  - successful");
    }
    if verb {
        println!("djinn:  setting up hosts file bind mount");
    }

    if mount::mount::<str, str, str, str>(
        Some("/run/djinn.hosts"),
        "/etc/hosts",
        None,
        mount::MsFlags::MS_BIND,
        None,
    )
    .is_err()
    {
        return Err(io::Error::last_os_error());
    }

    if verb {
        println!("djinn:  - successful");
    }
    Ok(())
}

fn _saveenv(verb: bool) -> io::Result<()> {
    // dump environment variables
    if verb {
        println!("djinn:  dumping wsl environment")
    }

    // read the WSL variables
    let mut out = String::from("INSIDE_DJINN=true\n");
    for key in ENVARS.iter() {
        match env::var(key) {
            Ok(val) => out.push_str(&format!("{}={}\n", key, val)),
            Err(e) => println!("djinn:  - missing variable {} -> {}", key, e),
        }
    }

    // save them
    fs::write("/run/djinn.env", out)?;

    if verb {
        println!("djinn:  - successful");
    }
    Ok(())
}

fn _get_systemd_pid() -> io::Result<unistd::Pid> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();

    if let Some(proc_) = sys
        .get_process_by_name("systemd")
        .iter()
        .filter(|p| p.name() == "systemd" && p.uid == 0)
        .min_by_key(|p| p.start_time())
    {
        return Ok(unistd::Pid::from_raw(proc_.pid()));
    }

    Ok(unistd::Pid::from_raw(0))
}

fn _get_env(envars: &mut Vec<ffi::CString>, envnames: &mut String) {
    let mut envnamesv = vec![String::from("TERM")];
    envars.push(ffi::CString::new(format!("TERM={}", env::var("TERM").unwrap())).unwrap());
    let data = fs::read_to_string("/run/djinn.env").unwrap_or_default();
    if data.is_empty() {
        println!("djinn: ? missing wsl envars")
    }
    for line in data.lines() {
        envars.push(ffi::CString::new(line).unwrap());
        envnamesv.push(line[..line.chars().position(|p| p == '=').unwrap()].to_string());
    }
    envnames.push_str(&envnamesv.join(","));
}

fn _grab_root(verb: bool) {
    let root_uid = unistd::Uid::from_raw(0);
    let root_gid = unistd::Gid::from_raw(0);
    _jump_user(verb, root_uid, root_gid);
}

fn _jump_user(verb: bool, user_id: unistd::Uid, group_id: unistd::Gid) {
    if verb {
        println!(
            "djinn: jumping - {}:{} -> {}:{}",
            unistd::getuid(),
            unistd::getgid(),
            user_id,
            group_id
        )
    }
    if let Err(e) = unistd::setresgid(group_id, group_id, group_id) {
        panic!(format!("failed to set gid {} -> {}", group_id, e));
    }
    if let Err(e) = unistd::setresuid(user_id, user_id, user_id) {
        panic!(format!("failed to set uid {} -> {}", user_id, e));
    }
}
