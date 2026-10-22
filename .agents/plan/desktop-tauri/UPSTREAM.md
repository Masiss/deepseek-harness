# Upstream tracking process (M9)

The native client does not inherit upstream UX automatically. Each upstream
release requires a parity pass.

## Per release
1. Diff the upstream web client's changed screens against PARITY.md.
2. For each changed screen: update the native implementation or mark it
   "lagging" in PARITY.md with a reason.
3. Run the probe suite on the native app.
4. Update PARITY.md checkboxes.

## Rules
- Never silently drop a screen; either implement or record the lag.
- A screen only counts as done when it runs on the native app.
- RSS gate (< 300 MiB) is re-measured per release.
