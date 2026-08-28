# System Health

`system-health` is the checked-in UI Strand example. It reads the same bounded
host metrics Helix already shows on Overview, then renders them in an isolated
page. It cannot see files, run commands, or call the privileged broker.

```text
helixctl strand check examples/strands/system-health
helixctl strand pack examples/strands/system-health -o system-health.strand.zip
```

Install the zip from the dashboard **Strands** page. Another owner can drop the
same zip onto their Helix; there is no Helix-operated store.
