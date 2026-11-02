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

# Combine reductions already observed to preserve the crash independently.
combined = widths.replace("; 16]", "; 1]")
combined = combined.replace("        let hash = hasher.finish();", "        let hash = 0;")
combined = combined.replace("        if self.retained_bytes() > SHARD_BYTES {\n            *self = Self::default();\n        }", "")
case = out / "10-combined"
case.mkdir(parents=True, exist_ok=True)
(case / "widths.rs").write_text(combined)
(case / "main.rs").write_text((out / "02-only-panic/main.rs").read_text())

standalone = {
    "11-mutex-map": '''
use std::{collections::HashMap, sync::Mutex};
#[derive(Default)]
struct Cache(Mutex<HashMap<u64, f32>>);
impl Cache {
    fn measure(&self, uncached: impl FnOnce() -> f32) -> f32 {
        if let Some(width) = self.0.lock().unwrap().get(&0).copied() { return width; }
        let width = uncached();
        self.0.lock().unwrap().insert(0, width);
        width
    }
}
fn main() {
    let cache = Cache::default();
    let failed = std::panic::catch_unwind(|| cache.measure(|| panic!("measurement failed")));
    assert!(failed.is_err());
    println!("real ok");
}
''',
    "12-mutex-option": '''
use std::sync::Mutex;
#[derive(Default)]
struct Cache(Mutex<Option<f32>>);
impl Cache {
    fn measure(&self, uncached: impl FnOnce() -> f32) -> f32 {
        if let Some(width) = *self.0.lock().unwrap() { return width; }
        let width = uncached();
        *self.0.lock().unwrap() = Some(width);
        width
    }
}
fn main() {
    let cache = Cache::default();
    let failed = std::panic::catch_unwind(|| cache.measure(|| panic!("measurement failed")));
    assert!(failed.is_err());
    println!("real ok");
}
''',
    "13-mutex-unit": '''
use std::sync::Mutex;
fn measure(cache: &Mutex<()>, uncached: impl FnOnce() -> f32) -> f32 {
    drop(cache.lock().unwrap());
    uncached()
}
fn main() {
    let cache = Mutex::new(());
    let failed = std::panic::catch_unwind(|| measure(&cache, || panic!("measurement failed")));
    assert!(failed.is_err());
    println!("real ok");
}
''',
    "14-callback-only": '''
fn measure(uncached: impl FnOnce() -> f32) -> f32 { uncached() }
fn main() {
    let failed = std::panic::catch_unwind(|| measure(|| panic!("measurement failed")));
    assert!(failed.is_err());
    println!("real ok");
}
''',
    "15-panic-only": '''
fn main() {
    let failed = std::panic::catch_unwind(|| panic!("measurement failed"));
    assert!(failed.is_err());
    println!("real ok");
}
''',
    "16-unused-map": '''
use std::collections::HashMap;
fn main() {
    let cache: HashMap<u64, String> = HashMap::new();
    std::hint::black_box(&cache);
    let failed = std::panic::catch_unwind(|| panic!("measurement failed"));
    assert!(failed.is_err());
    println!("real ok");
}
''',
}
standalone["17-option-explicit-guard"] = standalone["12-mutex-option"].replace(
    "        if let Some(width) = *self.0.lock().unwrap() { return width; }",
    "        let cached = { let guard = self.0.lock().unwrap(); *guard };\n        if let Some(width) = cached { return width; }",
)
standalone["18-option-array"] = standalone["12-mutex-option"].replace(
    "#[derive(Default)]\nstruct Cache(Mutex<Option<f32>>);",
    "struct Cache([Mutex<Option<f32>>; 16]);\nimpl Default for Cache { fn default() -> Self { Self(std::array::from_fn(|_| Mutex::new(None))) } }",
).replace("self.0.lock()", "self.0[0].lock()")

for name, source in standalone.items():
    case = out / name
    case.mkdir(parents=True, exist_ok=True)
    (case / "main.rs").write_text("#![forbid(unsafe_code)]\n" + source)
print(f"Generated {len(cases) + 1 + len(standalone)} diagnostic cases in {out}")
