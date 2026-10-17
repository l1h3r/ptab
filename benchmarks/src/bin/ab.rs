//! A/B driver for the `ab` bench. Builds a baseline and a candidate `ptab`, runs both `ab`
//! binaries in alternating processes, and reports each row's ratio with an interval.
//!
//! Run from `benchmarks/` so its `.cargo/config.toml` applies:
//!
//! ```text
//! cargo run --bin ab -- --base crates                 # crates.io release vs the working tree
//! cargo run --bin ab -- --base git:<rev>              # any revision vs the working tree
//! cargo run --bin ab -- --self-test --quick           # the same revision on both sides
//! ```
//!
//! SPEC is `crates` (the crates.io release), `tree` (the working tree) or `git:REV` (a detached
//! worktree of REV under `target/scratch/`). `--base` defaults to `crates`, `--cand` to `tree`.
//! `--quick` runs 3 rounds with one multi-thread count; `--full` (the default) runs 10 rounds with
//! every row. `--self-test` builds one revision twice and exits 1 when a gated row's ratio interval
//! leaves its band.
//!
//! Exit: 0 after a report, 1 when `--self-test` fails, 2 on bad usage or a failed build.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Allowed ratio band for single-thread rows, as a fraction of the baseline.
const BAND_SINGLE: f64 = 0.02;

/// Allowed ratio band for multi-thread rows.
const BAND_MULTI: f64 = 0.05;

/// Performance cores on the reference machine. Rows with more threads are reported as noisy.
const PCORES: usize = 8;

/// Nominal coverage of each reported interval.
const CONFIDENCE: f64 = 0.95;

const QUICK_ROUNDS: usize = 3;
const FULL_ROUNDS: usize = 10;

// -----------------------------------------------------------------------------
// Options
// -----------------------------------------------------------------------------

struct Options {
  base: String,
  cand: String,
  quick: bool,
  rounds: usize,
  self_test: bool,
}

fn parse_args() -> Result<Options, String> {
  let mut base: Option<String> = None;
  let mut cand: Option<String> = None;
  let mut quick: bool = false;
  let mut rounds: Option<usize> = None;
  let mut self_test: bool = false;

  let mut args = env::args().skip(1);

  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--base" => base = Some(args.next().ok_or("--base needs a SPEC")?),
      "--cand" => cand = Some(args.next().ok_or("--cand needs a SPEC")?),
      "--rounds" => {
        let value: String = args.next().ok_or("--rounds needs a count")?;
        let count: usize = value
          .parse()
          .map_err(|_| format!("bad --rounds `{value}`"))?;
        if count == 0 {
          return Err("--rounds must be at least 1".into());
        }
        rounds = Some(count);
      }
      "--quick" => quick = true,
      "--full" => quick = false,
      "--self-test" => self_test = true,
      other => return Err(format!("unknown argument `{other}`")),
    }
  }

  let (base, cand) = if self_test {
    ("git:HEAD".to_owned(), "git:HEAD".to_owned())
  } else {
    (
      base.unwrap_or_else(|| "crates".to_owned()),
      cand.unwrap_or_else(|| "tree".to_owned()),
    )
  };

  let rounds: usize = rounds.unwrap_or(if quick { QUICK_ROUNDS } else { FULL_ROUNDS });

  Ok(Options {
    base,
    cand,
    quick,
    rounds,
    self_test,
  })
}

// -----------------------------------------------------------------------------
// Git, sources and builds
// -----------------------------------------------------------------------------

/// Runs `git` in `dir` and returns its trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  let out = Command::new("git")
    .current_dir(dir)
    .args(args)
    .output()
    .map_err(|err| format!("cannot run git: {err}"))?;

  if !out.status.success() {
    let stderr = String::from_utf8_lossy(&out.stderr);
    return Err(format!("git {}: {}", args.join(" "), stderr.trim()));
  }

  Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// A directory this run created under `target/scratch/`, removed at the end.
enum Created {
  Worktree(PathBuf),
  Archive(PathBuf),
}

/// Returns the directory holding `ptab`'s source for `spec`, or `None` for crates.io.
fn resolve_source(
  root: &Path,
  scratch: &Path,
  spec: &str,
  created: &mut Vec<Created>,
) -> Result<Option<PathBuf>, String> {
  if spec == "crates" {
    return Ok(None);
  }

  if spec == "tree" {
    return Ok(Some(root.to_path_buf()));
  }

  let rev: &str = spec
    .strip_prefix("git:")
    .ok_or_else(|| format!("bad SPEC `{spec}`: use crates, tree or git:REV"))?;

  let sha: String = git(
    root,
    &[
      "rev-parse",
      "--verify",
      "--quiet",
      &format!("{rev}^{{commit}}"),
    ],
  )?;
  let dir: PathBuf = scratch.join(format!("base-{}", &sha[..12]));

  if dir.exists() {
    return Ok(Some(dir));
  }

  // A worktree needs write access to `.git`; `git archive` is the fallback when it is refused.
  match git(
    root,
    &["worktree", "add", "--detach", &dir.to_string_lossy(), &sha],
  ) {
    Ok(_) => created.push(Created::Worktree(dir.clone())),
    Err(err) => {
      eprintln!("ab: git worktree add failed ({err}); using git archive");

      fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;

      let status = Command::new("sh")
        .arg("-c")
        .arg(format!(
          "git archive {sha} | tar -x -C '{}'",
          dir.to_string_lossy()
        ))
        .current_dir(root)
        .status()
        .map_err(|err| format!("cannot run sh: {err}"))?;

      if !status.success() {
        return Err(format!("git archive {sha} failed"));
      }

      created.push(Created::Archive(dir.clone()));
    }
  }

  Ok(Some(dir))
}

/// Copies `src` into `dst`, replacing nothing that is not in `src`.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
  fs::create_dir_all(dst).map_err(|err| format!("cannot create {}: {err}", dst.display()))?;

  let entries = fs::read_dir(src).map_err(|err| format!("cannot read {}: {err}", src.display()))?;

  for entry in entries {
    let entry = entry.map_err(|err| format!("cannot read {}: {err}", src.display()))?;
    let from = entry.path();
    let to = dst.join(entry.file_name());

    if entry.file_type().map_err(|err| err.to_string())?.is_dir() {
      copy_tree(&from, &to)?;
    } else {
      fs::copy(&from, &to).map_err(|err| format!("cannot copy {}: {err}", from.display()))?;
    }
  }

  Ok(())
}

/// Stages the bench crate as a standalone workspace at `ws`, with the bench profile the root
/// workspace uses. The staged `Cargo.lock` is left alone, so later runs resolve offline.
fn stage(bench_dir: &Path, ws: &Path) -> Result<(), String> {
  fs::create_dir_all(ws).map_err(|err| format!("cannot create {}: {err}", ws.display()))?;

  for dir in ["benches", "src", ".cargo"] {
    let dst = ws.join(dir);

    if dst.exists() {
      fs::remove_dir_all(&dst).map_err(|err| format!("cannot clear {}: {err}", dst.display()))?;
    }

    copy_tree(&bench_dir.join(dir), &dst)?;
  }

  // The driver is not part of the measured crate; keep it out of the staged build.
  let driver = ws.join("src").join("bin");
  if driver.exists() {
    fs::remove_dir_all(&driver)
      .map_err(|err| format!("cannot clear {}: {err}", driver.display()))?;
  }

  let manifest = fs::read_to_string(bench_dir.join("Cargo.toml"))
    .map_err(|err| format!("cannot read Cargo.toml: {err}"))?;

  let staged = format!(
    "{manifest}\n[workspace]\n\n[profile.bench]\nstrip = \"symbols\"\nlto = \"fat\"\ncodegen-units = 1\n"
  );

  fs::write(ws.join("Cargo.toml"), staged).map_err(|err| format!("cannot write manifest: {err}"))
}

/// Stages one side and builds its `ab` bench. Returns the executable's path.
fn build_side(
  bench_dir: &Path,
  scratch: &Path,
  role: &str,
  spec: &str,
  source: Option<&Path>,
) -> Result<PathBuf, String> {
  let name: String = format!("{role}-{}", slug(spec));
  let ws: PathBuf = scratch.join(format!("ws-{name}"));

  stage(bench_dir, &ws)?;
  build(&ws, &scratch.join(format!("ab-{name}")), source)
}

/// Builds the `ab` bench in workspace `ws` for `source` (or crates.io) into `target`.
fn build(ws: &Path, target: &Path, source: Option<&Path>) -> Result<PathBuf, String> {
  let mut cmd = Command::new("cargo");

  cmd.current_dir(ws).env("CARGO_TARGET_DIR", target).args([
    "bench",
    "--no-run",
    "--bench",
    "ab",
    "--message-format=json",
  ]);

  if let Some(source) = source {
    cmd.arg("--config").arg(format!(
      "patch.crates-io.ptab.path=\"{}\"",
      source.display()
    ));
  }

  let out = cmd
    .output()
    .map_err(|err| format!("cannot run cargo: {err}"))?;

  if !out.status.success() {
    let stderr = String::from_utf8_lossy(&out.stderr);
    return Err(format!("build into {} failed:\n{stderr}", target.display()));
  }

  let json = String::from_utf8_lossy(&out.stdout);
  let key = "\"executable\":\"";

  json
    .lines()
    .filter(|line| {
      line.contains("\"kind\":[\"bench\"]")
        && line.contains("\"name\":\"ab\"")
        && line.contains(key)
    })
    .find_map(|line| {
      let start = line.find(key)? + key.len();
      let rest = &line[start..];
      let end = rest.find('"')?;
      Some(PathBuf::from(&rest[..end]))
    })
    .ok_or_else(|| "cargo did not report an ab executable".to_owned())
}

// -----------------------------------------------------------------------------
// Runs
// -----------------------------------------------------------------------------

struct Row {
  name: String,
  threads: usize,
  ns: f64,
}

/// Parses the TSV the `ab` bench prints, skipping its header.
fn parse_rows(text: &str) -> Result<Vec<Row>, String> {
  let mut rows: Vec<Row> = Vec::new();

  for line in text.lines().filter(|line| !line.starts_with("row\t")) {
    let mut fields = line.split('\t');

    let name = fields.next().ok_or("empty row line")?.to_owned();
    let threads = fields
      .next()
      .and_then(|value| value.parse().ok())
      .ok_or_else(|| format!("bad threads in `{line}`"))?;
    let ns = fields
      .next()
      .and_then(|value| value.parse().ok())
      .ok_or_else(|| format!("bad ns_per_op in `{line}`"))?;

    rows.push(Row { name, threads, ns });
  }

  Ok(rows)
}

/// Runs one `ab` process. Returns its rows and how many of its workers reported
/// `user-interactive` out of how many reported at all.
fn run_once(binary: &Path, quick: bool) -> Result<(Vec<Row>, usize, usize), String> {
  let mode: &str = if quick { "--quick" } else { "--full" };

  let out = Command::new(binary)
    .arg(mode)
    .output()
    .map_err(|err| format!("cannot run {}: {err}", binary.display()))?;

  if !out.status.success() {
    let stderr = String::from_utf8_lossy(&out.stderr);
    return Err(format!("{} failed:\n{stderr}", binary.display()));
  }

  let rows = parse_rows(&String::from_utf8_lossy(&out.stdout))?;

  let stderr = String::from_utf8_lossy(&out.stderr);
  let workers: Vec<&str> = stderr
    .lines()
    .filter(|line| line.starts_with("qos worker"))
    .collect();
  let interactive: usize = workers
    .iter()
    .filter(|line| line.ends_with("user-interactive"))
    .count();

  Ok((rows, interactive, workers.len()))
}

// -----------------------------------------------------------------------------
// Statistics
// -----------------------------------------------------------------------------

/// Probability that the median of `n` samples lies in `[s[k-1], s[n-k]]`.
fn coverage(n: usize, k: usize) -> f64 {
  let mut tail: f64 = 0.0;
  let mut choose: f64 = 1.0;

  for i in 0..k {
    tail += choose;
    choose = choose * (n - i) as f64 / (i + 1) as f64;
  }

  1.0 - 2.0 * tail / 2f64.powi(n as i32)
}

#[derive(Clone, Copy)]
struct Stat {
  lo: f64,
  med: f64,
  hi: f64,
}

/// Median and the widest order-statistic interval whose coverage reaches `CONFIDENCE`. With few
/// rounds the interval is the full range, and its coverage is stated by `coverage`.
fn summarize(mut values: Vec<f64>) -> Stat {
  values.sort_by(f64::total_cmp);

  let n: usize = values.len();
  let med: f64 = if n % 2 == 1 {
    values[n / 2]
  } else {
    (values[n / 2 - 1] + values[n / 2]) / 2.0
  };

  let mut best: usize = 1;

  for k in 1..=n / 2 {
    if coverage(n, k) >= CONFIDENCE {
      best = k;
    }
  }

  Stat {
    lo: values[best - 1],
    med,
    hi: values[n - best],
  }
}

#[derive(Default)]
struct Cell {
  base: Vec<f64>,
  cand: Vec<f64>,
}

/// What a row's ratio interval says about the band.
#[derive(Clone, Copy, PartialEq)]
enum Verdict {
  /// More threads than P-cores: reported, not gated.
  Noisy,
  /// The interval lies entirely above (`true`) or below (`false`) the band: a reported change.
  Change { slower: bool },
  /// The interval is inside the band and narrower than it: no change, and the band is resolved.
  Resolves,
  /// The interval is inside the band but as wide as the band or wider: no change claimed.
  Cannot,
}

impl Verdict {
  fn label(self, band: f64) -> String {
    match self {
      Self::Noisy => "noisy, not gated".to_owned(),
      Self::Change { slower: true } => "change: slower".to_owned(),
      Self::Change { slower: false } => "change: faster".to_owned(),
      Self::Resolves => format!("resolves {:.0}%, no change", band * 100.0),
      Self::Cannot => format!("cannot resolve {:.0}%", band * 100.0),
    }
  }
}

/// The band for a row: single-thread rows are tighter than multi-thread rows.
fn band_for(threads: usize) -> f64 {
  if threads == 1 {
    BAND_SINGLE
  } else {
    BAND_MULTI
  }
}

/// Classifies a row's ratio interval against its band.
fn verdict(threads: usize, ratio: Stat) -> Verdict {
  if threads > PCORES {
    return Verdict::Noisy;
  }

  let band: f64 = band_for(threads);

  if ratio.lo > 1.0 + band {
    Verdict::Change { slower: true }
  } else if ratio.hi < 1.0 - band {
    Verdict::Change { slower: false }
  } else if (ratio.hi - ratio.lo) / 2.0 < band {
    Verdict::Resolves
  } else {
    Verdict::Cannot
  }
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn run(opts: &Options) -> Result<bool, String> {
  let bench_dir: PathBuf = env::current_dir().map_err(|err| format!("no working dir: {err}"))?;

  if !bench_dir.join("benches/ab.rs").is_file() {
    return Err("run from benchmarks/ (benches/ab.rs not found)".into());
  }

  let root_text: String = git(&bench_dir, &["rev-parse", "--show-toplevel"])?;
  let root: PathBuf = PathBuf::from(root_text);
  let scratch: PathBuf = root.join("target").join("scratch");

  fs::create_dir_all(&scratch).map_err(|err| format!("cannot create scratch: {err}"))?;

  // Each side builds in its own standalone workspace under scratch, so patching `ptab` never
  // touches the tree's Cargo.lock (or its workspace members).
  let mut created: Vec<Created> = Vec::new();

  let built: Result<(PathBuf, PathBuf), String> = (|| {
    let base_src = resolve_source(&root, &scratch, &opts.base, &mut created)?;
    let cand_src = resolve_source(&root, &scratch, &opts.cand, &mut created)?;

    let base_bin = build_side(
      &bench_dir,
      &scratch,
      "base",
      &opts.base,
      base_src.as_deref(),
    )?;
    let cand_bin = build_side(
      &bench_dir,
      &scratch,
      "cand",
      &opts.cand,
      cand_src.as_deref(),
    )?;

    Ok((base_bin, cand_bin))
  })();

  let result: Result<Outcome, String> =
    built.and_then(|(base_bin, cand_bin)| interleave(opts, &scratch, &base_bin, &cand_bin));

  cleanup(&root, created);

  let (cells, qos_ok, qos_total) = result?;

  let stats: Vec<(String, usize, Stat, Stat, Stat, Verdict)> = cells
    .into_iter()
    .map(|((name, threads), cell)| {
      let ratios: Vec<f64> = cell
        .base
        .iter()
        .zip(&cell.cand)
        .map(|(base, cand)| cand / base)
        .collect();

      let base: Stat = summarize(cell.base);
      let cand: Stat = summarize(cell.cand);
      let ratio: Stat = summarize(ratios);
      let verdict: Verdict = verdict(threads, ratio);

      (name, threads, base, cand, ratio, verdict)
    })
    .collect();

  let mut table: String = String::new();

  let _ = writeln!(
    table,
    "rounds {}  base {}  cand {}  bands {:.0}% single / {:.0}% multi  qos user-interactive {qos_ok}/{qos_total} workers",
    opts.rounds,
    opts.base,
    opts.cand,
    BAND_SINGLE * 100.0,
    BAND_MULTI * 100.0,
  );

  let _ = writeln!(
    table,
    "{:<12} {:>3}  {:>25}  {:>25}  {:>22}  {:>9}  verdict",
    "row", "thr", "base ns/op [lo, hi]", "cand ns/op [lo, hi]", "ratio [lo, hi]", "half-width"
  );

  let mut failing: Vec<String> = Vec::new();

  for (name, threads, base, cand, ratio, verdict) in &stats {
    let half: f64 = (ratio.hi - ratio.lo) / 2.0 * 100.0;

    let _ = writeln!(
      table,
      "{:<12} {:>3}  {:>9.3} [{:>7.3}, {:>7.3}]  {:>9.3} [{:>7.3}, {:>7.3}]  {:>6.3} [{:>5.3}, {:>5.3}]  {:>8.2}%  {}",
      name,
      threads,
      base.med,
      base.lo,
      base.hi,
      cand.med,
      cand.lo,
      cand.hi,
      ratio.med,
      ratio.lo,
      ratio.hi,
      half,
      verdict.label(band_for(*threads)),
    );

    if matches!(verdict, Verdict::Change { .. }) {
      failing.push(format!("{name}/{threads}"));
    }
  }

  print!("{table}");

  if opts.self_test {
    if failing.is_empty() {
      println!(
        "self-test: PASS, no gated row reports a change (no interval lies entirely outside its band)"
      );
      return Ok(true);
    }

    println!(
      "self-test: FAIL, rows report a change on identical code: {}",
      failing.join(", ")
    );
    return Ok(false);
  }

  Ok(true)
}

/// A filesystem-safe label for a SPEC.
fn slug(spec: &str) -> String {
  spec
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
    .collect()
}

type Outcome = (BTreeMap<(String, usize), Cell>, usize, usize);

/// Runs both binaries for every round, alternating which side goes first. Also writes the raw
/// samples to `scratch`.
fn interleave(opts: &Options, scratch: &Path, base: &Path, cand: &Path) -> Result<Outcome, String> {
  let mut cells: BTreeMap<(String, usize), Cell> = BTreeMap::new();
  let mut raw: Vec<String> = Vec::new();
  let mut qos_ok: usize = 0;
  let mut qos_total: usize = 0;

  for round in 0..opts.rounds {
    let order: [(&str, &Path); 2] = if round % 2 == 0 {
      [("base", base), ("cand", cand)]
    } else {
      [("cand", cand), ("base", base)]
    };

    for (side, binary) in order {
      let (rows, ok, total) = run_once(binary, opts.quick)?;

      qos_ok += ok;
      qos_total += total;

      for row in rows {
        raw.push(format!(
          "{side}\t{round}\t{}\t{}\t{:.4}",
          row.name, row.threads, row.ns
        ));

        let cell: &mut Cell = cells.entry((row.name, row.threads)).or_default();

        match side {
          "base" => cell.base.push(row.ns),
          _ => cell.cand.push(row.ns),
        }
      }
    }

    eprintln!("ab: round {} of {} done", round + 1, opts.rounds);
  }

  let stamp: u64 = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);

  let path = scratch.join(format!("ab-{stamp}.tsv"));
  let mut text: String = String::from("side\tround\trow\tthreads\tns_per_op\n");

  for line in &raw {
    text.push_str(line);
    text.push('\n');
  }

  fs::write(&path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
  eprintln!("ab: raw samples in {}", path.display());

  Ok((cells, qos_ok, qos_total))
}

/// Removes what this run created. Worktrees go through plain `git worktree remove`.
fn cleanup(root: &Path, created: Vec<Created>) {
  for item in created {
    match item {
      Created::Worktree(dir) => {
        if let Err(err) = git(root, &["worktree", "remove", &dir.to_string_lossy()]) {
          eprintln!("ab: could not remove worktree {}: {err}", dir.display());
        }
      }
      Created::Archive(dir) => {
        if let Err(err) = fs::remove_dir_all(&dir) {
          eprintln!("ab: could not remove {}: {err}", dir.display());
        }
      }
    }
  }
}

fn main() -> ExitCode {
  let opts: Options = match parse_args() {
    Ok(opts) => opts,
    Err(err) => {
      eprintln!("ab: {err}");
      eprintln!(
        "usage: ab [--base SPEC] [--cand SPEC] [--quick|--full] [--rounds N] [--self-test]"
      );
      return ExitCode::from(2);
    }
  };

  match run(&opts) {
    Ok(true) => ExitCode::SUCCESS,
    Ok(false) => ExitCode::from(1),
    Err(err) => {
      eprintln!("ab: {err}");
      ExitCode::from(2)
    }
  }
}
