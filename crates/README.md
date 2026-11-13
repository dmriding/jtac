# jtac library crates

An internal part of [jtac](https://github.com/dmriding/jtac), the gate that
signs a record of what it checked before a push. It is on crates.io only so
that `cargo install jtac` resolves.

No stability promise: the API changes with any release, and every crate in
the workspace is versioned in lockstep with the `jtac` binary. Depend on the
binary's behaviour and its versioned evidence and policy formats, not on
this crate.

The design, with each crate's concern, is
[DESIGN.md](https://github.com/dmriding/jtac/blob/main/DESIGN.md) section 8.1.
