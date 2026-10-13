# Bundled typography

SF Pro / SF Rounded / SF Mono are not redistributed. The bundled substitutes used by Linux and all fixtures are Inter 4.1 (400/500/600/700) for UI, Nunito Bold (700) for explicitly rounded badges, and
JetBrains Mono 2.304 Regular for code and Bold for pairing codes. Font sizes remain the Swift logical point sizes. Outlines and
metrics differ from SF; this is a documented typography constraint, not exact SF parity.

- Inter: unmodified `extras/ttf/Inter-{Regular,Medium,SemiBold,Bold}.ttf` from
  https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip, with `LICENSE.txt` (SIL OFL 1.1).
- Nunito: https://github.com/google/fonts/tree/8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5/ofl/nunito,
  `Nunito[wght].ttf` instantiated with `fonttools varLib.instancer Nunito.ttf wght=700 -o Nunito-Bold.ttf`.
  No outline modifications. `Nunito-OFL.txt` is the upstream SIL OFL 1.1 licence.
- JetBrains Mono: unmodified `fonts/ttf/JetBrainsMono-{Regular,Bold}.ttf` from
  https://github.com/JetBrains/JetBrainsMono/tree/cd5227bd1f61dff3bbd6c814ceaf7ffd95e947d9 (v2.304),
  with `JetBrainsMono-OFL.txt` (SIL OFL 1.1).

Use `theme::font(size, Weight)`; `.strong()` alone does not select the bundled semibold outline.
The Windows app prefers `%WINDIR%/Fonts/SegUIVar.ttf` when installed, with explicit variable
weights 400/500/600/700 and optical size 12. Missing, invalid or unsupported fonts fall back to
the bundled faces. Segoe is loaded locally and is never redistributed. Rounded and monospaced
faces stay bundled. Fixtures and tests call `theme::install` to keep the bundled typography
deterministic; production calls `theme::install_native`. Fonts are installed once per egui
context before the first layout.
