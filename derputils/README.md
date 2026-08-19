# derputils

A set of utils which deserve questioning
what's purpose of their existence in the first place.

Multicall binary (busybox-style): invoked as `derputils` it dispatches on
the first argument; invoked through a symlink (or copy) named after an
applet it runs that applet directly.

    ln -s derputils qr
    ln -s derputils uuid7

Applets: `qr` (QR code from stdin/clipboard), `uuid7` (print a UUIDv7).

