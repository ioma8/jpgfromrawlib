# Memory-mapped extraction cleanup

## Context and objective

Preserve the existing memory-mapped embedded-JPEG extraction route as the default
for every input. Make the surrounding library smaller, easier to maintain, and
less allocation-heavy. Decoding and external tools are fallbacks only, after
embedded extraction has been exhausted.

This is an implementation checklist, not a record of completed changes. Keep
checkboxes current during implementation and record measurements and deviations
in the execution log at the end.

## Non-negotiable behavior

- Keep `memmap2`, the platform mapping/prefetch implementation, metadata locators,
  `memchr` marker scanning, and missing-FF recovery in the default extraction path.
- Do not restrict extraction to an extension whitelist. An unrecognized extension
  must still be eligible for embedded-JPEG discovery.
- Do not dispatch generic raster files, GPR files, or other recognized extensions
  directly to a decoder before attempting mapped extraction.
- Preserve the current order: TIFF-header discovery; CR3, RAF, and TIFF embedded
  locators; ordinary JPEG marker scan; missing-FF recovery; then decoding fallbacks.
  Correct the invalid-candidate fallthrough described below without introducing a
  new preference order for valid candidates.
- Keep valid existing output bytes stable during structural cleanup. Isolate
  intentional correctness changes and their baseline changes in separate commits.
- Preserve crate-root `process_file_bytes`, `process_file`, `FindJpegType`, and
  `SUPPORTED_EXTENSIONS`, plus the existing deprecated compatibility module.
- Do not promise that every listed extension contains a usable preview. Failure
  to extract one is an ordinary result and does not justify implicit development
  of the RAW image in the minimal build.
- No new parser framework, decoder registry, thread-pool framework, custom SIMD,
  alternate I/O backend, or automatic cache.
- Changes to scan order, mmap advice, prefetch, scheduling, or data structures need
  representative before/after measurements. Revert experiments that do not help.

## Evidence from the initial audit

The existing release audit ran on the ten local Sony ARW files on 2026-09-06.
These are warm-cache medians on one machine, not portable performance targets.

| Stage | Median per file |
| --- | ---: |
| TIFF embedded locator, Largest | 0.296 microseconds |
| Validate and assemble Largest | 13.804 microseconds |
| End-to-end Smallest | 26.404 microseconds |
| End-to-end Largest | 54.521 microseconds |
| Ordinary marker scan, Largest | 1,470.333 microseconds |
| Missing-FF scan, Largest | 1,456.514 microseconds |

The release library suite passed 18 tests, with two benchmark tests ignored.
The larger `sample_raws` corpus was absent. Its tests return successfully when
the corpus is absent; that must not be reported as broad format verification.

Current sources worth preserving:

- `src/parser/mod.rs`: mapped extraction and fallback ordering.
- `src/parser/jpeg.rs`: marker finders, EOI reuse, validation, JPEG assembly.
- `src/parser/tiff.rs` and `src/parser/bmff.rs`: bounded metadata traversal.
- `src/platform/{unix,windows}.rs`: platform-specific mapping and prefetch.

## Phase 1: Establish reproducible compatibility and performance baselines

Files: `src/parser/mod.rs`, `src/bin/perf_probe.rs`, `tests/`,
`perf_probe.sh`, `perf_probe.md`, `perf_artifacts/`.

- [ ] Record commit, OS, CPU, Rust version, enabled features, release settings,
  corpus file hashes, and benchmark commands alongside results.
- [ ] Run `cargo test --release --lib` and the existing integration suite before
  implementation. Distinguish failures from absent corpus/tool prerequisites.
- [ ] Run `cargo test --release --lib -- --ignored parsing_performance_audit --nocapture`.
- [ ] Run `cargo test --release --lib -- --ignored parallel_scaling_audit --nocapture`.
- [ ] Save output hashes for both Largest and Smallest on local real files. The
  existing perf probe only covers Smallest; extend it minimally to cover both.
- [ ] Add small synthetic inputs for TIFF, CR3, RAF, marker-only extraction,
  missing-FF extraction, and no-preview input. Assert selected payload, not just
  the presence of JPEG start/end markers.
- [ ] Exercise embedded extraction with a generic raster extension, `.gpr`, and
  an unknown extension to lock down extraction-before-decoding behavior.
- [ ] Separate parser-only in-memory measurements from mapped end-to-end bytes
  extraction and file-output measurements. Keep input loading outside timed
  parser-only sections.
- [ ] Record warm-cache timings separately from cold-cache or larger-than-cache
  experiments. Do not call a run cold merely because the process restarted.
- [ ] Use scratch output directories for measurements. Validate deletion targets
  in `perf_probe.sh`; do not overwrite the saved baseline as part of a normal run.

Acceptance: an unchanged revision can reproduce outputs and the approximate
timing distribution. Missing real formats are listed explicitly. No parser or
I/O optimization has been introduced in this phase.

## Phase 2: Repair scanner and candidate fallthrough behavior

Files: `src/parser/jpeg.rs`, `src/parser/mod.rs`, `src/parser/tiff.rs`,
`src/parser/bmff.rs`, related parser tests.

- [ ] In both ordinary and missing-FF marker loops, stop when no EOI exists at
  or after the current search position. Search positions only increase, so later
  candidates cannot succeed. Preserve the best candidate already found.
- [ ] Add regressions for many SOI markers with no EOI, and a valid JPEG followed
  by many unterminated candidates. Test Largest and Smallest.
- [ ] Benchmark geometrically increasing malformed buffers to verify that the
  unsuccessful EOI search no longer produces quadratic tail rescanning. Avoid
  brittle elapsed-time assertions in unit tests.
- [ ] Make the existing structured stages continue to the next existing stage
  when their candidate fails validation. In particular, an invalid CR3/RAF/TIFF
  candidate must not suppress ordinary marker recovery.
- [ ] Validate structured candidate ranges before accepting them as the winner.
  A malformed larger candidate must not discard a valid smaller candidate.
- [ ] In TIFF embedded selection, collect an IFD's fields first, then consider
  its candidate once. Preserve tag precedence and traversal behavior; capture
  orientation even when that tag follows the offset/length pair.
- [ ] Add focused malformed-range, invalid-winner, late-orientation, and valid
  recovery-candidate cases. Retain existing cycle and traversal limits.
- [ ] Run existing randomized scanner equivalence tests. Document any intentional
  correctness difference from the reference instead of weakening assertions.

Acceptance: valid baseline files retain identical output; new cases recover valid
embedded JPEGs without invoking decoders. Worst-case no-EOI scanning is bounded
by forward searches rather than repeatedly scanning the remaining tail.

## Phase 3: Make decoding and CLI dependencies optional

Files: `Cargo.toml`, `build.rs`, `src/fallback/`, `src/parser/tiff.rs`,
`src/main.rs`, `src/bin/perf_probe.rs`, feature-dependent integration tests.

Target feature layout:

| Feature | Enables |
| --- | --- |
| default: empty feature set | Existing mapped extraction and public async API |
| `cli` | Clap, Indicatif, and the existing command-line binaries |
| `tiff-preview` | Uncompressed RGB TIFF preview encoding fallback |
| `raster-fallback` | Existing base generic raster conversion codecs |
| `generic-raster` | `raster-fallback` plus existing AVIF/DDS/EXR coverage |
| `dcraw-fallback` | dcraw build/run, temporary files, PNM decode and JPEG encode |
| `gpr-fallback` | GPR build/run and temporary files |
| `full` | All fallback features; CLI remains explicitly selectable |

Keep `generic-raster` as an additive compatibility feature. Do not silently remove
its existing format coverage. Switching default fallback behavior is a documented
release behavior change even though public function signatures remain intact.

- [ ] Set `default = []`; gate every image-decoding/encoding branch and import.
  Embedded TIFF JPEG lookup remains unconditional; only RGB-to-JPEG conversion
  is gated behind `tiff-preview`.
- [ ] Make `image`, `tempfile`, `cc`, Clap, and Indicatif optional production/build
  dependencies, wired to the features that actually use them. Keep `tempfile`
  available to tests through dev-dependencies where needed.
- [ ] Retain `anyhow`, `byteorder`, `memchr`, `memmap2`, and required platform
  dependencies. Do not replace small useful dependencies solely to lower a count.
- [ ] Retain Tokio for the existing async file-output API and fallback scheduling
  compatibility. Remove unused Tokio feature flags only after checking binaries
  and tests. Removing Tokio entirely is outside this cleanup.
- [ ] Add explicit binary targets with `required-features = ["cli"]`. Update
  scripts and binary integration tests so a minimal library build does not build
  CLI dependencies or accidentally run a stale binary from `target/debug`.
- [ ] Gate native build functions and their invocation. With no fallback features,
  `build.rs` must not find compilers, invoke CMake, or build native executables.
- [ ] Preserve mapped extraction before all enabled fallback dispatch. If no
  enabled fallback applies, return a contextual no-preview error; never retry
  through a tool merely because of the file's extension.
- [ ] Preserve `SUPPORTED_EXTENSIONS` as the existing RAW extension list, not a
  feature-dependent extraction whitelist or a guarantee of successful decoding.
- [ ] Split or gate existing raster tests according to their codec features. Run
  embedded extraction tests in the minimal feature configuration.
- [ ] Compare `cargo tree --edges normal,build` and isolated clean build time for
  minimal and full configurations. Exclude dev-dependencies from production size
  claims. Verify minimal builds on a machine without native fallback prerequisites.

Acceptance: default builds can extract the same embedded JPEGs across all tested
formats without compiling image codecs, CLI dependencies, or native tools. Full
fallback functionality remains available through explicit features.

## Phase 4: Simplify orchestration without changing execution policy

Files: `src/extract.rs`, `src/parser/mod.rs`, `src/parser/jpeg.rs`,
`src/fallback/mod.rs`, `src/fallback/temp_dir.rs`, `src/main.rs`.

- [ ] Keep the public entry points in `extract.rs` and move file mapping plus
  fallback orchestration there. Leave format discovery in parser modules.
- [ ] Use a private synchronous helper over `&[u8]` for embedded discovery. Return
  validated offsets/length and the minimal assembly metadata needed for ordinary
  and repaired JPEGs. Reuse `EmbeddedJpegInfo` where practical.
- [ ] Keep map ownership, prefetch calls, and output lifetime management in the
  file-processing layer. Do not expose bare borrowed bytes after dropping the map.
- [ ] Preserve the existing async scheduling behavior during this structural
  move. Do not add a `spawn_blocking` hop to every successful extraction as an
  incidental refactor. A future scheduling change needs its own latency and
  executor-responsiveness measurements.
- [ ] After extraction fails, reuse the mapped bytes for generic decoding where
  practical. If a blocking task needs ownership, move the mapping into that task
  instead of reading the entire source again. Keep this implementation simple
  and verify map lifetime and Send requirements with the compiler.
- [ ] Release the mapping before external-tool fallbacks when they no longer use
  it, rather than retaining mapped input across an unrelated subprocess wait.
- [ ] Replace `TempDirGuard` with the existing `tempfile::TempDir` and builder.
- [ ] Remove the generic JPEG encoder's unnecessary `Cursor<Vec>` wrapper.
- [ ] Remove the CLI's redundant nested async block and outer application `Result`
  around `ProcessingResult`. Preserve join-error and per-file-error behavior.
- [ ] Remove stale internal reexports or forwarding helpers made unnecessary by
  the move. Keep the deprecated public compatibility module intact.

Acceptance: API contract tests and output hashes pass. Platform advice and
successful extraction scheduling are unchanged. Generic fallback no longer
requires a redundant full input read where mapped bytes are already available.

## Phase 5: Reduce output allocation and copying

Files: `src/parser/jpeg.rs`, `src/extract.rs`, `src/parser/tiff.rs`,
`src/fallback/generic.rs`, extraction and output tests.

- [ ] Track missing-FF candidates by range and validate their virtual leading
  prefix without copying every candidate. Share validation logic; do not create
  a second independently maintained JPEG parser.
- [ ] Assemble the selected repaired JPEG directly into its final output buffer.
  Avoid the current repaired-buffer allocation followed by another header copy.
- [ ] Keep `process_file_bytes` returning owned `Vec<u8>` as today. Reserve the
  exact final assembled size once, including the existing Exif header behavior.
- [ ] Add a private write path for `process_file` that consumes the same selected
  preview and writes header plus mapped payload without first building the full
  JPEG Vec. Correctly handle short writes if using vectored I/O; two complete
  writes are acceptable if simpler and faster in measurements.
- [ ] Keep output directory creation, file naming, errors, and overwrite behavior
  compatible. Ensure output aliasing the input cannot truncate a live mapped
  source; retain an owned-buffer path or reject the alias before opening output.
- [ ] Verify byte-for-byte equality between file output and bytes output for
  ordinary, oriented, and repaired JPEGs, including empty/truncated error cases.
- [ ] Use `DynamicImage::into_rgb8` when generic decoding owns the image, avoiding
  the unconditional clone caused by borrowing and calling `to_rgb8`.
- [ ] Evaluate borrowing a contiguous uncompressed TIFF RGB strip instead of
  assembling a new RGB Vec. Keep multi-strip behavior unchanged; only retain the
  special case if fallback memory/latency measurements justify the branch.
- [ ] Measure allocation count/bytes or peak memory, plus throughput for both
  output APIs and both selection modes. Keep changes only when they improve the
  intended workload without a material regression elsewhere.

Acceptance: mapped file output avoids a JPEG-sized assembly allocation where
possible; bytes output assembles once; existing output bytes remain stable.

## Phase 6: Make optional native fallback builds and deployment reliable

Files: `build.rs`, `src/fallback/dcraw.rs`, `src/fallback/gpr.rs`,
native fallback integration tests, `README.md`.

- [ ] Configure GPR CMake build type explicitly from the Cargo build profile and
  use `--config` for multi-configuration generators. Verify actual compiler flags
  in release builds; the audited release build had no GPR optimization flags.
- [ ] Verify dcraw receives the intended compiler optimization flags. Do not
  assume Cargo's Rust profile automatically configures every subprocess build.
- [ ] Preserve subprocess-based fallback isolation; do not introduce an FFI
  rewrite as part of this cleanup.
- [ ] Replace hidden reliance on compile-time absolute OUT_DIR paths with an
  explicit deployment contract: runtime tool-path overrides plus installed-tool
  lookup, with build-produced paths retained only as a development convenience.
- [ ] Document exact override names, lookup precedence, tool filenames, and how
  an application bundles or installs them. Pass paths as process arguments, not
  shell command strings, and report missing tools with actionable errors.
- [ ] Test fallback execution using explicit paths from a working directory
  unrelated to the checkout. Verify a deployed consumer does not require the
  original Cargo build directory when its tools are configured correctly.
- [ ] Verify platform target handling and executable locations on Windows, macOS,
  and Linux. Explicitly reject unsupported cross-compilation configurations rather
  than accidentally building or running a host executable as the target tool.

Acceptance: native tools are never built in the minimal configuration. Enabled
fallbacks have a tested deployment contract and optimized release builds.

## Phase 7: Verification, documentation, and release handoff

Files: `.github/workflows/ci.yml`, `README.md`, `perf_probe.md`, relevant tests.

- [ ] Run minimal tests on Linux, macOS, and Windows:
  `cargo test --no-default-features --lib --tests`.
- [ ] Check each fallback feature independently and test the complete combination:
  `cargo test --features full,cli`.
- [ ] Run `cargo fmt --all -- --check`, minimal Clippy, and
  `cargo clippy --all-targets --all-features -- -D warnings`, with native codec
  prerequisites installed for the full job.
- [ ] Preserve public API compile tests in both minimal and full configurations.
- [ ] Add real JPEG decoding assertions to selected corpus tests using a
  dev-dependency decoder. The existing `decode_jpeg` helper checks headers and
  must not be described as validating the complete compressed image.
- [ ] Make dedicated corpus CI fail when its required corpus is missing. Ordinary
  developer runs may skip external data, but print and report those skips clearly.
- [ ] Re-run saved performance commands with baseline and candidate under the
  same conditions, using repeated interleaved runs to distinguish noise from
  regressions. Report medians and spread; measure tail latency where sample count
  supports it. Re-run concurrency 1, 2, 4, and 8 for representative real files.
- [ ] Keep byte hashes unchanged for unaffected fixtures. For intentional fixes,
  record the old failure, expected output, and independently validated new result
  before updating only the affected baselines.
- [ ] Document default mmap-first behavior, optional fallback features, CLI build
  commands, blocking work inside the async API, and fallback deployment requirements.
- [ ] Replace the platform safety comment's CLI-only rationale with the actual
  library backing-file assumptions. Document concurrent input mutation/truncation
  limitations; do not claim that bounds checks alone make mutable mapped files safe.
- [ ] Document the release migration from implicit default decoders to opt-in
  `full` or individual fallback features. Choose the release version according to
  the project's compatibility policy; do not publish automatically.
- [ ] Record final production dependency differences, clean build measurements,
  extraction timings, memory results, tested formats, and remaining coverage gaps.

## Explicitly deferred work

- Reordering fast-path stages or replacing mmap with read/seek/pread.
- Removing Tokio, replacing the public async API, or adding a public borrowed
  preview API. The private byte-oriented helper does not require an API redesign.
- Changing Largest/Smallest semantics across format families. RAF currently uses
  its designated preview irrespective of selection mode; document and test that
  behavior before a separate semantic change.
- Changing Exif insertion/preservation policy. Current output unconditionally
  prepends an orientation header, including default orientation 1. Resolving
  duplicate Exif or orientation precedence requires dedicated fixtures and a
  separately reviewed output compatibility change.
- Adding new dedicated format parsers, alternate JPEG encoders, hand-written SIMD,
  new pools, or aggressive release-profile flags without workload evidence.
- Deleting vendored code or the older architecture plan merely because native
  fallbacks are optional. Packaging size changes need a separate exact scope.

## Completion criteria

- [ ] Current mapped embedded extraction remains first for every input extension.
- [ ] Minimal builds retain structured lookup, marker scanning, and missing-FF repair.
- [ ] Decoders and native tools run only after embedded extraction is exhausted.
- [ ] Minimal production dependency graph excludes codecs, CLI packages, and native
  fallback build dependencies; retained Tokio compatibility is documented.
- [ ] Existing public signatures and unaffected output bytes are preserved.
- [ ] No-EOI tail rescanning and invalid-candidate recovery have regression coverage.
- [ ] Copy reductions have measured benefit; scan/mmap behavior has no unexplained
  regression on the tested corpus.
- [ ] Full fallback builds, deployment, and feature combinations are verified.
- [ ] Final report distinguishes tested formats, skipped tests, measured results,
  and remaining limitations. No unmeasured speedup claim is made.

## Execution log

For each phase, append: commit/reference, changes completed, commands and feature
configuration tested, result artifact paths, measured differences, and deviations
with their reason. Leave implementation checkboxes pending until actually verified.

### 2026-09-06 working tree

- Reference: working tree after the initial release audit; no commit was created.
- Completed: empty default feature set; optional CLI, raster, TIFF preview, dcraw,
  and GPR fallbacks; conditional native builds; explicit CLI binary features;
  mmap-first structured and marker extraction; forward-only no-EOI scanner exit;
  invalid structured candidate fallthrough; late TIFF orientation capture;
  mapped ordinary file output; `TempDir` use; runtime native-tool overrides;
  generic encoder ownership; unknown-extension and output-equality regressions.
- Verified with `cargo fmt --all -- --check`, minimal release tests, full
  `cargo test --features full,cli`, minimal and full Clippy with `-D warnings`,
  independent fallback feature checks, API trybuild tests, and both ignored
  release audits. The local Sony ARW corpus produced unchanged SHA-256 output
  hashes in the CLI probe. The generic and broader RAW corpora are not present.
- Measurements: release parser audit medians after cleanup were 0.217 us for
  TIFF lookup, 14.008 us for validation plus assembly, 1.476 ms for marker
  scanning, and 1.456 ms for missing-FF scanning. The end-to-end audit measured
  3.788 ms Smallest and 3.836 ms Largest on this run; these are local warm-cache
  measurements and are retained as a follow-up investigation because they are
  slower than the original recorded end-to-end medians. The clean isolated
  checks took 4.16 s minimal and 33.55 s full with CLI/all targets. `/usr/bin/time
  -l` reported peak RSS of 102,891,520 bytes for the bytes probe and 372,506,624
  bytes for the file-output probe.
- Deviations: `cc` was removed instead of retained as an optional build
  dependency because the build script must compile without it in the minimal
  graph; dcraw uses the configured C compiler directly. Cross-platform runtime
  execution and deployment were not available on this machine. The benchmark
  shell script was hardened, but the optional hyperfine run was skipped because
  `hyperfine` is not installed.

### 2026-09-06 implementation audit

- Added during audit: Largest output generation in `perf_probe`, CI minimal/full
  jobs, real JPEG decoding through the dev-only `jpeg-decoder`, GPR/generic
  raster extraction-before-fallback tests, RAF invalid-candidate recovery,
  late-orientation coverage, input/output alias protection, and all-target lint
  verification.
- Final checks: `cargo test --no-default-features --lib --tests`,
  `cargo test --features full,cli`, `cargo clippy --all-targets --all-features
  -- -D warnings`, format, and diff checks all pass. The probe now emits 20
  outputs for ten Sony files; Smallest hash for `sony_a7_iii_01` remains
  `71d5e7bbe05147a3b35d9bd4bfdb7d1f7efcb5bddf70ec68ebb5c30355f40f04`, and the
  Largest hash is `0121c39ed28b351b39f88eaf3c72bf6255965c18956e0aacf3c6746fd698b870`.
- Additional verification: release `parsing_performance_audit` and
  `parallel_scaling_audit` pass; the minimal production dependency tree contains
  only the retained core dependencies; a malformed larger TIFF candidate now has
  a dedicated recovery regression; `/usr/bin/time -l` recorded peak RSS for both
  bytes and file-output probes; and isolated clean minimal/full build timings are
  recorded above. The optional `cc` build dependency was intentionally omitted:
  direct compiler invocation keeps it out of the minimal build script while
  still applying explicit dcraw optimization flags.
- Regression investigation: the first post-cleanup end-to-end measurement was
  invalid because `map_embedded_jpeg` eagerly evaluated the ordinary marker scan
  while constructing a chained iterator, even after a structured TIFF candidate
  would have succeeded. The marker scan is now lazy. A repeat release audit
  measured 23.950 us Smallest and 53.858 us Largest, compared with the initial
  26.404 us and 54.521 us medians; this is approximately 9% and 1% faster,
  respectively. The parser-stage timings remain comparable.
- Remaining plan items requiring unavailable or additional workload evidence:
  cross-platform execution and deployment tests, contiguous TIFF-strip borrowing
  evaluation, repeated interleaved hyperfine runs (`hyperfine` is not installed),
  and dedicated corpus verification for formats absent from this checkout. No
  claim is made for those items.
