# Requests for comment

An RFC proposes a change to `DESIGN.md`. It is required when a change
touches a shipped format, the verdict function, policy, the command
classifier, signing, storage, a hook, or a decision listed in `DESIGN.md`
section 21. Bug fixes, new table rows, and anything that implements the
design as written do not need one.

Process:

1. Copy `0000-template.md` to `NNNN-short-title.md` with the next number and
   open a pull request containing only the RFC. Status `draft`.
2. Discussion happens on that pull request. The author updates the text.
3. The maintainer merges it as `accepted` or closes it as `rejected`. An
   accepted RFC is not a change yet.
4. The implementing pull request edits `DESIGN.md`, adds an ADR in
   `docs/adr/` where the RFC's "Decision" section says one is needed, and
   links the RFC. Shipped formats are never edited: a change is a new
   version with a reader for the old one.

| # | Title | Status |
| --- | --- | --- |
