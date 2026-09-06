# HTTPS probe

This Strand shows how to wire an external HTTPS API into Helix. It can store an
Authorization header in namespaced Strand storage, call Open-Meteo through
Helix's origin-allowlisted client, and remember the last response. Swap the
origin and fetch URL for a vacuum, battery, printer, or any other HTTPS JSON API.

```text
helixctl strand check examples/strands/https-probe
helixctl strand pack examples/strands/https-probe -o https-probe.strand.zip
```
