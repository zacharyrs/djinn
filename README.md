# djinn (beta?)

An opinionated rust knockoff of [`genie`](https://github.com/arkane-systems/genie)

note: **wsl2** is required!

## why

i wanted to try learn some [`rust`](https://www.rust-lang.org/), and `dotnet` is a [pain on arch](https://www.reddit.com/r/archlinux/comments/cx64r5/the_state_of_net_core_on_arch/)

## usage

```none
djinn 0.1.0
Zachary Riedlshah <git@zacharyrs.me>
an opinionated rust knockoff of genie

USAGE:
    djinn [FLAGS] <SUBCOMMAND>

FLAGS:
    -v               Set verbosity
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    init     set up a bottle
    shell    launch a shell inside bottle, sets up a bottle if necessary
    run      run a command inside bottle, sets up a bottle if necessary
```

it's pretty much the same as [`genie`](https://github.com/arkane-systems/genie#usage)

## requirements

like [`genie`](https://github.com/arkane-systems/genie), `djinn` requires a few things...

- `dbus`
- `policykit-1`
- [`daemonize`](http://software.clapper.org/daemonize/)

note: the first two are likely included in your os

unlike [`genie`](https://github.com/arkane-systems/genie), `djinn` doesn't need...

- `dotnet`
- `hostess`

## installation

1. drop `djinn` into `/usr/local/bin/`, or `/usr/bin/`
2. make sure it's owned as `root`, `chown root:root <path to djinn>`
3. give it a `setuid` bit, `chmod +s <path to djinn>`

note: `systemd` environment generators

to enable wsl environment variables within the systemd environment,
you may wish to add `/usr/local/lib/systemd/system-environment-generators/10-djinn.sh`

```bash
#!/bin/sh
if [ -e /run/djinn.env ]
then
  cat /run/djinn.env
fi
```

## caveats

unfortunately there's still a few problems `djinn` can't solve...

in particular, the same bugs from [`genie`](https://github.com/arkane-systems/genie#bugs)

> - breaks `/proc` based tools
> - it's clunky

additionally, but not mentioned for [`genie`](https://github.com/arkane-systems/genie#bugs), there isn't a clean way to kill the bottle

you can call `shutdown`, but `systemd` tries to unmount disks and breaks things...

`wsl --shutdown` works though
