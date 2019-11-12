#![warn(clippy::all)]
use std::env;
use std::ffi;
use std::fs;
use std::io::{self};
use std::path;
use std::process::Command;
use std::thread;
use std::time::Duration;

extern crate clap;
use clap::{
    app_from_crate, crate_authors, crate_description, crate_name, crate_version, AppSettings, Arg,
    ArgGroup, SubCommand,
};

extern crate nix;
use nix::{mount, unistd};

extern crate sysinfo;
use sysinfo::{ProcessExt, SystemExt};

static SUFFIX: &str = "-wsl";
static ENVARS: [&str; 3] = ["WSL_DISTRO_NAME", "WSL_INTEROP", "WSLENV"];

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
            SubCommand::with_name("net")
                .about("network related functions")
                .setting(AppSettings::ArgRequiredElseHelp)
                .setting(AppSettings::DeriveDisplayOrder)
                .setting(AppSettings::DisableHelpSubcommand)
                .args(&[
                    Arg::with_name("vm_ip").help("return the ip of the wsl linux client"),
                    Arg::with_name("vm_subnet").help("return the subnet of the linux client"),
                    Arg::with_name("win_ip").help("return the ip of the wsl windows host"),
                    Arg::with_name("win_subnet").help("return the subnet of the windows host"),
                ])
                .group(
                    ArgGroup::with_name("parameter")
                        .args(&["vm_ip", "vm_subnet", "win_ip", "win_subnet"])
                        .required(true),
                ),
            // TODO reenable bottle reversion when functional
            // SubCommand::with_name("revert").about("destruct our bottle"),
        ])
        .get_matches();

    let verb: bool = opts.is_present("verbose");

    if let Some(_subopts) = opts.subcommand_matches("net") {
        if _subopts.is_present("vm_ip") {
            println!("vm_ip");
        }
        if _subopts.is_present("win_ip") {
            println!("win_ip");
        }
        if _subopts.is_present("vm_subnet") {
            println!("vm_subnet");
        }
        if _subopts.is_present("win_subnet") {
            println!("win_subnet");
        }
        return;
    }

    if unistd::geteuid() != unistd::Uid::from_raw(0) {
        println!("! djinn needs to be run as root - try the setuid bit");
        return;
    }

    if !fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap()
        .to_lowercase()
        .contains("microsoft")
    {
        println!("! djinn must be run within wsl");
        return;
    }

    for line in fs::read_to_string("/proc/self/mounts").unwrap().lines() {
        let mnt: Vec<_> = line.split_whitespace().collect();
        if mnt[1] == "/" {
            if mnt[2] == "lxfs" {
                println!("! djinn only supports wsl2");
                return;
            } else {
                break;
            }
        }
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

    if let Some(_subopts) = opts.subcommand_matches("revert") {
        _grab_root(verb);
        revert(verb);
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
        let mut command: Vec<ffi::CString> = vec![];
        for i in subopts.values_of("command").unwrap() {
            command.push(ffi::CString::new(i).unwrap())
        }
        if bottle_inside {
            println!("djinn: no need to enter bottle - we're in it already");
            _get_env(&mut envars, &mut envnames);
            unistd::execvpe(&command[0], &command, &envars).expect("failed to launch shell");
            return;
        }
        _grab_root(verb);
        if !bottle_exists {
            systemd_pid = init(verb);
        }
        _get_env(&mut envars, &mut envnames);
        run(verb, systemd_pid, &user_name, &envars, command);
        _jump_user(verb, user_id, group_id);
        return;
    }
}

fn init(verb: bool) -> unistd::Pid {
    // init a bottle
    if verb {
        println!("djinn: beginning bottle init...");
    }

    let hostname = _backup_hostname(verb).expect("failed to backup hostname");
    _backup_hosts(verb).expect("failed to backup hosts");

    let new_hostname = format!("{}{}", hostname, SUFFIX);

    _set_hostname(verb, &new_hostname).expect("failed to set custom hostname");
    _set_hosts(verb).expect("failed to prepare hosts");
    _patch_hosts(verb, &hostname, &new_hostname).expect("failed to set custom hosts");

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

fn revert(verb: bool) {
    // destroy a bottle
    if verb {
        println!("djinn: beginning bottle destruction...");
    }

    let hostname = _backup_hostname(verb).expect("failed to get original hostname");
    let new_hostname = format!("{}{}", hostname, SUFFIX);
    _patch_hosts(verb, &new_hostname, &hostname).expect("failed to revert hosts patches");

    _cleanup(verb).expect("failed to cleanup");
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

    let mut args = vec![];
    for &i in [
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
    {
        args.push(ffi::CString::new(i).unwrap());
    }

    unistd::execve(&args[0], &args, envars).expect("failed to launch shell");
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

    let mut args = vec![];
    for &i in [
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
    {
        args.push(ffi::CString::new(i).unwrap());
    }

    args.append(&mut command);

    unistd::execve(&args[0], &args, &envars).expect("failed to launch shell");
}

fn _backup_hostname(verb: bool) -> io::Result<String> {
    // dump existing hostname
    if verb {
        println!("djinn:  backing up hostname");
    }

    // check if hostname backed up -> only backup if not currently
    let hostname: String;
    if !path::Path::new("/run/djinn.hostname.orig").exists() {
        hostname = fs::read_to_string("/etc/hostname")?;
        fs::write("/run/djinn.hostname.orig", &hostname)?;
    } else {
        if verb {
            println!("djinn:  - hostname already backed up, not overwriting");
        }
        hostname = fs::read_to_string("/run/djinn.hostname.orig")?;
    }

    // return either the current (just backed up),
    // or the previously backed up hostname
    if verb {
        println!("djinn:  - successful");
    }
    Ok(hostname.trim().to_string())
}

fn _backup_hosts(verb: bool) -> io::Result<()> {
    // dump existing hosts
    if verb {
        println!("djinn:  backing up hosts");
    }

    // check if hosts backed up -> only backup if not currently
    if !path::Path::new("/run/djinn.hosts.orig").exists() {
        let hosts: String = fs::read_to_string("/etc/hosts")?;
        fs::write("/run/djinn.hosts.orig", &hosts)?;
    } else if verb {
        println!("djinn:  - hosts already backed up, not overwriting");
    }

    if verb {
        println!("djinn:  - successful");
    }
    Ok(())
}

fn _set_hostname(verb: bool, hostname: &str) -> io::Result<()> {
    if verb {
        println!("djinn:  setting hostname via bind mount -> {}", hostname);
    }

    // save our custom hostname
    fs::write("/run/djinn.hostname", format!("{}\n", hostname))?;

    if let Err(_e) = mount::mount::<str, str, str, str>(
        Some("/run/djinn.hostname"),
        "/etc/hostname",
        None,
        mount::MsFlags::MS_BIND,
        None,
    ) {
        return Err(io::Error::last_os_error());
    }

    if verb {
        println!("djinn:  - successful");
    }
    Ok(())
}

fn _set_hosts(verb: bool) -> io::Result<()> {
    if verb {
        println!("djinn:  setting up hosts file bind mount");
    }

    let hosts: String = fs::read_to_string("/etc/hosts")?;
    fs::write("/run/djinn.hosts", hosts)?;

    if let Err(_e) = mount::mount::<str, str, str, str>(
        Some("/run/djinn.hosts"),
        "/etc/hosts",
        None,
        mount::MsFlags::MS_BIND,
        None,
    ) {
        return Err(io::Error::last_os_error());
    }

    if verb {
        println!("djinn:  - successful");
    }
    Ok(())
}

fn _patch_hosts(verb: bool, old: &str, new: &str) -> io::Result<()> {
    if verb {
        println!("djinn:  patching hosts: {} -> {}", old, new);
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

fn _cleanup(verb: bool) -> io::Result<()> {
    if verb {
        println!("djinn:  cleaning up")
    }

    if let Err(_e) = mount::umount::<str>("/etc/hosts") {
        return Err(io::Error::last_os_error());
    }

    if let Err(_e) = mount::umount::<str>("/etc/hostname") {
        return Err(io::Error::last_os_error());
    }

    if verb {
        println!("djinn:  - bind mounts removed")
    }

    fs::copy("/run/djinn.hosts", "/etc/hosts")?;
    fs::remove_file("/run/djinn.env")?;
    fs::remove_file("/run/djinn.hosts")?;
    fs::remove_file("/run/djinn.hosts.orig")?;
    fs::remove_file("/run/djinn.hostname")?;
    fs::remove_file("/run/djinn.hostname.orig")?;

    if verb {
        println!("djinn:  - files removed")
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
