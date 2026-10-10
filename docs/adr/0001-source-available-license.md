# 0001. License under FSL-1.1-ALv2, not an OSI license

Date: 2026-10-10
Status: accepted

## Context

jtac is published for anyone to use, read, and fork, and the maintainer
intends to offer commercial licenses to organisations. The design draft named
AGPL-3.0-or-later. AGPL permits a competitor to sell a fork as a product or
service provided they publish their source, which does not match the intent.
No OSI-approved license can forbid that.

## Decision

License under the Functional Source License, version 1.1, with the Apache 2.0
future grant (SPDX `FSL-1.1-ALv2`). Any use is permitted except offering the
software or a derivative as a competing commercial product or service. Each
release converts to Apache 2.0 two years after it is made available. The
README states plainly that this is source-available, not open source.

## Alternatives

- AGPL-3.0 plus a commercial license: true open source, but allows the
  competing fork.
- BUSL-1.1 with an additional use grant: can charge for internal production
  use, which conflicts with `DESIGN.md` principle 11 and adds friction for the
  users the project wants.
- Elastic License 2.0, SSPL, PolyForm Shield: comparable restrictions without
  the time-limited conversion that keeps goodwill.

## Consequences

Internal use at any organisation is free. Competing products need a license
from the maintainer. The project must not describe itself as open source.
Every release date starts a two-year clock to Apache 2.0, so the conversion
is a feature to publicise, not a loophole to manage.
