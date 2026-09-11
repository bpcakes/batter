# Architecture decisions

These decisions govern the source implementation and future work; they do not
establish that the current source has passed validation.

| ADR | Decision |
| --- | --- |
| [001](001-native-rust.md) | Native Rust composition, not an Effect port. |
| [002](002-lifetimes-and-cleanup.md) | Explicit lifetimes, phased shutdown, conservative cleanup. |
| [003](003-replay-and-budgets.md) | One total budget and explicit replay authorization. |
| [004](004-optional-adapters.md) | Optional adapters with one-way upstream dependency ownership. |
| [005](005-errors-and-telemetry.md) | Concrete domain errors; aggregate internal reports; sanitized telemetry. |
| [006](006-workspace-packages.md) | Virtual workspace, separate adoption packages, and a shared dispatch seam. |
| [007](007-unix-platform-scope.md) | Unix-only platform scope; Windows is unsupported and not planned. |
| [008](008-http-transport-ownership.md) | Observe response, body, connection and direct server lifetimes separately. |
| [009](009-http-lifetime-observations.md) | Measured HTTP/1.1 lifetimes; preserve response-construction ownership. |
| [010](010-agent-only-consumption.md) | Optimize the canonical integration path for coding-agent consumers. |

Changes to these decisions require a new or amended ADR, corresponding contract
tests, and an update to the validation/status documents. Do not silently weaken
them in the name of convenience or compiler compatibility.
