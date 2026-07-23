# Secure pre-merge release rehearsal

## Status

Accepted for issue #319.

## Context

Release-profile changes such as LTO and codegen-unit tuning must be measured before merge. The production `Release Build` workflow cannot build feature-branch source safely: its macOS and Linux jobs receive Developer ID, notarization, and updater-signing credentials, and its caches are owned by trusted `main` builds.

Allowing branch-controlled source into that workflow would expose release credentials and let unmerged code write trusted release caches. Notarization latency is also external queue time, so it obscures the Rust compile/link effect that release-profile experiments are intended to measure.

## Decision

Use a separate `Release Rehearsal` workflow with these boundaries:

- The workflow is manually dispatched from `main`, which supplies the trusted workflow definition.
- An exact 40-character source commit SHA is the only build-source input. Every build job checks out that immutable SHA without persisted Git credentials.
- Cache-owning composite actions are checked out separately at the trusted workflow SHA; source commits cannot redefine cache keys or save policy.
- Source code runs only in jobs with read-only repository permission and no repository or release secrets.
- macOS measures the unsigned release app build; Linux measures unsigned deb and AppImage builds. These are the accepted proxies for compile, link, and bundle performance because LTO does not affect signing or notarization.
- Cargo and CUDA cache namespaces include the source SHA. A prime run may write only those isolated keys; a second run measures the warm state.
- JSON artifacts record the trusted workflow SHA, source SHA, run identity, build duration, cache state where available, and binary/package sizes.
- The rehearsal cannot tag, publish, generate updater manifests, or trigger the production promotion workflow.

## Consequences

Pre-merge performance decisions use the warm rehearsal's critical build leg, not notarization or overall production-release wall time. Production `Release Build` remains the authority for signing, updater behavior, notarization, Gatekeeper, and publication. A change that improves the rehearsal still must preserve `strip = false`, updater bundle markers, production policy tests, and normal signed-release validation.
