## What this changes

<!-- One or two sentences. Link the issue if there is one. -->

## Checklist

- [ ] `npm run check` passes locally (Rust fmt, clippy and tests; app typecheck, lint and tests).
- [ ] New or changed behaviour has a test that runs offline. Simulator tests use ngspice or are marked to run only when the simulator is present.
- [ ] No API keys, vendor model files or PDK content in the diff.
- [ ] `docs/TOOLS.md` regenerated with `cargo run -p aispice -- docs tools` if tools changed.
- [ ] README, docs or `CHANGELOG.md` updated if behaviour changed.
