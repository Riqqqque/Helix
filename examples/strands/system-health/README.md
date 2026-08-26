# System Health reference Strand

This directory demonstrates the small, reviewable metadata surface created by
the Strand Kit preview. Validate it from the Helix repository root:

```text
cargo run --locked -p helixctl -- strand check examples/strands/system-health
```

It requests one narrowly named capability and explains why it needs it. The
resource values are ceilings, not reservations.

There is intentionally no runnable component here. Helix does not have a Strand
runtime, SDK, or stable host API yet, so adding pretend extension code would
make the example misleading.
