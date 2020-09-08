# djinn (beta?)

An opinionated rust knockoff of [`genie`](https://github.com/arkane-systems/genie)

note: **wsl2** is required!

## why

i wanted to try play with [`rust`](https://www.rust-lang.org/), and `dotnet` is a [pain on arch](https://www.reddit.com/r/archlinux/comments/cx64r5/the_state_of_net_core_on_arch/)

## usage

```none
djinn 0.3.0
Zachary Riedlshah <git@zacharyrs.me>
an opinionated rust knockoff of genie

USAGE:
    djinn [FLAGS] <SUBCOMMAND>

FLAGS:
    -v               Set verbosity
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    init       set up a bottle
    shell      launch a shell inside bottle, sets up a bottle if necessary
    run        run a command inside bottle, sets up a bottle if necessary
    cleanup    destroy the bottle bottle
```

it's pretty much the same as [`genie`](https://github.com/arkane-systems/genie) but in rust

note: the subcommands infer, meaning you can just write `i`, `s`, `r`, and `c`.

## requirements

like [`genie`](https://github.com/arkane-systems/genie), `djinn` requires a few things...

- `dbus`
- `policykit-1`
- [`daemonize`](http://software.clapper.org/daemonize/)

note: the first two are likely included in your os

unlike [`genie`](https://github.com/arkane-systems/genie), `djinn` doesn't need...

- `dotnet`
- ~~`hostess`~~ (not necessary in new releases of genie)

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
> - it's clunky (less than dotnet though!)

## future work

though i'm sure i'll come up with something new to add, right now it's just...

- [ ] a [configuration system](https://github.com/zacharyrs/djinn/issues/2) like `genie`
