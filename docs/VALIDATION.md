# Extraction checks (2026-10-25)

- All new Python sources compiled through the parser; no automated tests/fixtures added.
- Native byte-preserved files match the extraction manifest digests.
- New Core package imports contain no experiment/top-level script dependencies.
- Native and new Core CLI help execute without starting an engine or training.
- Wheel built with the existing setuptools84.2.0; unpacked-wheel imports succeed
  under isolated Python outside the research source tree. Both native and route
  .mjs bridges are present; no experiment/integration/game packages are included.
- Original width1 BC was loaded by its unchanged strict loader against independent
  expected file/data hashes; all681429 parameters were explicitly adopted, saved
  with the new source-bound schema and reloaded with identical parameter values.
  Zero training updates were performed. This does not establish new forward,
  gradient or gameplay parity, throughput or strategy quality.
- No game simulation, gradient comparison, performance repeat or long run executed.
- Parallel collection and bridge observation/headless optimizations remain future work.

The initial pip-wheel command could not run because the selected interpreter has
no pip module. Building through its installed setuptools.build_meta succeeded;
no dependency installation or environment mutation was needed.
