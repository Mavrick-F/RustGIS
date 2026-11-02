"""Generate a bounded reduction batch from the current width cache; diagnostic branch only."""

from pathlib import Path

root = Path(__file__).resolve().parents[1]
out = root / "target" / "arm64-reduction"
widths = (root / "crates/render/src/text/widths.rs").read_text()
widths = widths.replace("#[cfg(test)]\nmod tests;", "")

reentrant = 'assert_eq!(cache.measure("same", 10.0, false, || cache.measure("same", 10.0, false, || 17.0)), 17.0);'
failed = 'let failed = std::panic::catch_unwind(|| cache.measure("failed", 10.0, false, || panic!("measurement failed")));'
check = 'assert!(failed.is_err());'
retry = 'assert_eq!(cache.measure("failed", 10.0, false, || 23.0), 23.0);'
cached = 'assert_eq!(cache.measure("failed", 10.0, false, || panic!("retry must have been cached")), 23.0);'
basic = 'assert_eq!(cache.measure("same", 10.0, false, || 17.0), 17.0);'
original = [reentrant, failed, check, retry, cached]
without_reentry = [failed, check, retry, cached]
cases = {
    "00-original": (widths, original),
    "01-no-reentry": (widths, without_reentry),
    "02-only-panic": (widths, [failed, check]),
    "03-only-reentry": (widths, [reentrant]),
    "04-no-cached-retry": (widths, [failed, check, retry]),
    "05-basic-before-panic": (widths, [basic, failed, check, retry, cached]),
    "06-drop-panic-early": (widths, [reentrant, failed, check, "drop(failed);", retry, cached]),
    "07-one-shard": (widths.replace("; 16]", "; 1]"), without_reentry),
    "08-no-budget": (
        widths.replace("        if self.retained_bytes() > SHARD_BYTES {\n            *self = Self::default();\n        }", ""),
        without_reentry,
    ),
    "09-no-hashing": (
        widths.replace("        let hash = hasher.finish();", "        let hash = 0;"),
        without_reentry,
    ),
}

for name, (source, steps) in cases.items():
    case = out / name
    case.mkdir(parents=True, exist_ok=True)
    (case / "widths.rs").write_text(source)
    main = '#![allow(dead_code, unused_imports)]\n#![forbid(unsafe_code)]\nmod widths;\nuse widths::Cache;\nfn main() {\n    let cache: Cache = Cache::default();\n'
    main += "\n".join("    " + step for step in steps)
    main += '\n    println!("real ok");\n}\n'
    (case / "main.rs").write_text(main)
print(f"Generated {len(cases)} diagnostic cases in {out}")
