# Security

jtac is a gate. A way to get evidence for work it did not check is a vulnerability,
whether or not it reads like one.

**Report privately** through GitHub's advisory form for this repository:
https://github.com/dmriding/jtac/security/advisories/new. Do not open a public
issue, and do not post a working bypass anywhere public before a fix ships.

In scope: any way to make a check look like it ran when it did not, to obtain
a `CLEARED HOT` or a signed root for a tree that was not checked, to forge or
alter evidence, to reach a lever from an agent tool surface, or to make a hook
print anything but its protocol on stdout.

A rule that misses a change it should flag, such as a new suppression spelling,
is an ordinary bug, since the evidence still records what ran: open an issue
with the missed-rule form.

Out of scope, by design (DESIGN.md section 6): a compromised operating system
or toolchain on the developer's machine, malicious code inside a wrapped
analyzer, and bypass by a developer who removes jtac or pushes with
`--no-verify`. The last is recorded as absent coverage, not prevented.

You will get an acknowledgement within seven days and a fix or a stated
decision within ninety. Reporters are credited in the release notes unless
they ask not to be.
