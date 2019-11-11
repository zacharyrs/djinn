# djinn

An opinionated rust knockoff of [genie](https://github.com/arkane-systems/genie)

## todo

- [ ] check for root
- [ ] check for existing bottle

### on init

- [x] backup /etc/hosts (-> /run/djinn.hosts.orig) and /etc/hostname (-> /run/djinn.hostname.orig)
- [x] read backed up hostname, append suffix for new -> store as /run/djinn.hostname
- [x] patch hosts (/run/djinn.hostname.orig to /run/djinn.hostname) -> store as /run/djinn.hosts
- [x] bind mount /run/djinn.hostname onto /etc/hostname and /run/djinn.hosts onto /etc/hosts
- [ ] _set transient hostname?_
- [x] backup environment variables -> store as /run/djinn.env

### on revert

- [x] patch hosts (/run/djinn.hostname to /run/djinn.hostname.orig) -> store as /run/djinn.hosts
- [x] unmount /etc/hosts and /etc/hostname
- [x] _copy /run/djinn.hosts to /etc/hosts -> restores any other changes to hosts?_
- [ ] _restore transient hostname_
