# Batch 1 schema errata review

Base: `5b7dca1`. Changes implement E1–E9 from the accepted batch 3 proposal.

Read-only Codex review found one P2: the proposed physical-camera aperture
formula used millimetres for the legacy pixel blur radius. Corrected the
annotation to convert focal length to pixels using frame height and sensor
height before dividing by twice fStop. This is contract documentation for
B3-F; this commit does not change the legacy renderer.

Verification (Flatpak SDK 25.08): all 42 1.0-valid repository fixtures also
validate against the revised 1.1 XSD. Pre-change Release: 53/53 CTests pass.
`schema.compatibility` now repeats the complete fixture check in CTest.
No existing renderer arithmetic or golden files changed.
