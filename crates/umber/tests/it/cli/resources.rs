//! Path resolution, distribution mirrors, profiling, and input receipts.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_input_through_texinputs_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX input search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let search_dir = temp_dir.path().join("generic/hyphen");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX input search directory");
    let source = job_dir.join("plain.tex");
    fs::write(&source, "\\input hyphen \\message{after-hyphen}\\end\n")
        .expect("write principal input");
    fs::write(search_dir.join("hyphen.tex"), "\\message{loaded-hyphen}\n")
        .expect("write searched input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run input search smoke");

    assert!(
        output.status.success(),
        "input search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("loaded-hyphen"));
    assert!(stdout.contains("after-hyphen"));
}

#[cfg(feature = "profiling")]
#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the profiling binary.
fn profiling_stats_flag_reports_feature_only_census() {
    let temp_dir = tempfile::tempdir().expect("create profiling CLI temp dir");
    let source = temp_dir.path().join("profiling.tex");
    fs::write(&source, "\\def\\profilemacro{A}\\profilemacro\\end\n")
        .expect("write profiling CLI fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--profiling-stats")
        .arg(&source)
        .output()
        .expect("run profiling-enabled umber binary");
    let stderr = String::from_utf8(output.stderr).expect("profiling stderr is utf-8");
    assert!(output.status.success(), "{stderr}");
    for prefix in [
        "EXPANSION_STATS ",
        "EXPANSION_TIMERS_NS ",
        "HOT_CORE_CENSUS ",
        "RETAINED_GENERATION_CENSUS ",
        "SAVE_JOURNAL_CENSUS ",
    ] {
        assert_eq!(stderr.matches(prefix).count(), 1, "{prefix}: {stderr}");
    }
    assert!(
        stderr.contains("\"expansion_opcodes\":{\"macro\":1"),
        "the profiling census must observe the fixture's macro expansion: {stderr}"
    );

    let census: serde_json::Value = serde_json::from_str(
        stderr
            .lines()
            .find_map(|line| line.strip_prefix("HOT_CORE_CENSUS "))
            .expect("hot-core census line"),
    )
    .expect("valid census JSON");
    let object_sum = |name: &str| {
        census[name]
            .as_object()
            .expect("census object")
            .values()
            .map(|value| value.as_u64().expect("counter"))
            .sum::<u64>()
    };
    assert_eq!(
        object_sum("main_control_meanings"),
        object_sum("command_families")
    );
    assert_eq!(
        census["dispatch_opcodes"]["unexpandable_primitives"]
            .as_object()
            .expect("unexpandable opcode object")
            .values()
            .map(|value| value.as_u64().expect("counter"))
            .sum::<u64>(),
        census["main_control_meanings"]["unexpandable_primitive"]
            .as_u64()
            .expect("unexpandable meaning count")
    );
}

#[cfg(not(feature = "profiling"))]
#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the shipping binary.
fn profiling_stats_flag_is_absent_from_the_shipping_resolution() {
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--profiling-stats", "unused.tex"])
        .output()
        .expect("run ordinary umber binary");

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_cold_offline_local_mirror_resolves_positive_and_negative_file_requests() {
    let temp_dir = tempfile::tempdir().expect("create distribution temp dir");
    let source = temp_dir.path().join("main.tex");
    let distribution = temp_dir.path().join("distribution");
    let cache = temp_dir.path().join("cache");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create distribution");
    fs::write(
        &source,
        concat!(
            "\\openin0=optional.cfg ",
            "\\ifeof0 \\message{optional-absent}",
            "\\else \\errmessage{unexpected optional file}\\fi ",
            "\\input remote \\message{after-remote}\\end\n",
        ),
    )
    .expect("write source");
    let remote = b"\\message{from-distribution}\n";
    let object_digest = hex_ahash64(remote);
    let object = format!("ahash64-v1-{object_digest}");
    fs::write(objects.join(&object), remote).expect("write object");
    let first_shard = format!(
        "{{\"schema\":3,\"distribution\":\"test-snapshot\",\"index\":0,\"files\":{{\"tex:remote.tex\":{{\"virtualPath\":\"/texlive/tex/remote.tex\",\"object\":\"{object}\",\"ahash64\":\"{object_digest}\",\"bytes\":{}}}}}}}\n",
        remote.len()
    );
    let second_shard =
        "{\"schema\":3,\"distribution\":\"test-snapshot\",\"index\":1,\"files\":{}}\n";
    let first_shard = pack_shard(&ManifestShard::parse(&first_shard).expect("first shard"))
        .expect("packed first shard");
    let second_shard = pack_shard(&ManifestShard::parse(second_shard).expect("second shard"))
        .expect("packed second shard");
    let first_shard_digest = hex_ahash64(&first_shard);
    let second_shard_digest = hex_ahash64(&second_shard);
    fs::write(
        objects.join(format!("ahash64-v1-{first_shard_digest}")),
        first_shard,
    )
    .expect("write first shard");
    fs::write(
        objects.join(format!("ahash64-v1-{second_shard_digest}")),
        second_shard,
    )
    .expect("write second shard");
    let manifest = format!(
        "{{\"schema\":8,\"distribution\":\"test-snapshot\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":1,\"shardCount\":2,\"shards\":[\"{first_shard_digest}\",\"{second_shard_digest}\"]}}\n"
    );
    fs::write(distribution.join("manifest-v8.json"), &manifest).expect("write manifest");
    let manifest_digest = hex_ahash64(manifest.as_bytes());

    let first = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", &cache)
        .args(["run", "--show-fixtures", "--offline", "--distribution"])
        .arg(&distribution)
        .args(["--distribution-ahash64", &manifest_digest])
        .arg(&source)
        .output()
        .expect("run cold local distribution");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    assert!(first_stdout.contains("optional-absent"));
    assert!(first_stdout.contains("from-distribution"));
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr UTF-8"),
        "umber: acquired 1 distribution resource(s)\n"
    );

    fs::remove_file(objects.join(object)).expect("remove source object after warming");
    let second = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", &cache)
        .args(["run", "--show-fixtures", "--offline", "--distribution"])
        .arg(&distribution)
        .args(["--distribution-ahash64", &manifest_digest])
        .arg(&source)
        .output()
        .expect("run warm offline distribution");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(second.stderr.is_empty());
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(second_stdout.contains("optional-absent"));
    assert!(second_stdout.contains("from-distribution"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_rejects_a_manifest_that_mismatches_its_pin() {
    let temp_dir = tempfile::tempdir().expect("create manifest mismatch temp dir");
    let source = temp_dir.path().join("main.tex");
    let manifest = temp_dir.path().join("manifest.json");
    fs::write(&source, "\\input absent \\end\n").expect("write source");
    fs::write(
        &manifest,
            "{\"schema\":2,\"distribution\":\"test\",\"objectsBaseUrl\":\"https://example.invalid/\",\"files\":{}}",
    )
    .expect("write manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("XDG_CACHE_HOME", temp_dir.path().join("cache"))
        .args([
            "run",
            "--distribution",
            manifest.to_str().expect("UTF-8 path"),
            "--distribution-ahash64",
            "0000000000000000",
        ])
        .arg(&source)
        .output()
        .expect("run mismatched manifest");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("distribution manifest digest mismatch")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_offline_local_mirror_miss_names_the_exact_object_digest() {
    let temp_dir = tempfile::tempdir().expect("create offline miss temp dir");
    let source = temp_dir.path().join("main.tex");
    let distribution = temp_dir.path().join("distribution");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create distribution");
    fs::write(&source, "\\input remote \\end\n").expect("write source");
    let bytes = b"\\relax\n";
    let digest = hex_ahash64(bytes);
    let entry = format!(
        "\"tex:remote.tex\":{{\"virtualPath\":\"/texlive/remote.tex\",\"object\":\"ahash64-v1-{digest}\",\"ahash64\":\"{digest}\",\"bytes\":{}}}",
        bytes.len()
    );
    let shard =
        format!("{{\"schema\":3,\"distribution\":\"test\",\"index\":0,\"files\":{{{entry}}}}}\n");
    let shard =
        pack_shard(&ManifestShard::parse(&shard).expect("typed shard")).expect("packed shard");
    let shard_digest = hex_ahash64(&shard);
    fs::write(objects.join(format!("ahash64-v1-{shard_digest}")), shard).expect("write shard");
    fs::write(
        distribution.join("manifest-v8.json"),
        format!(
            "{{\"schema\":8,\"distribution\":\"test\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
        ),
    )
    .expect("write manifest");
    let root = fs::read(distribution.join("manifest-v8.json")).expect("read manifest");
    let root_digest = hex_ahash64(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("XDG_CACHE_HOME", temp_dir.path().join("empty-cache"))
        .args(["run", "--offline", "--distribution"])
        .arg(&distribution)
        .args(["--distribution-ahash64", &root_digest])
        .arg(&source)
        .output()
        .expect("run offline miss");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to read ")
            && stderr.contains(&format!("ahash64-v1-{digest}"))
            && !stderr.contains("tex:remote.tex"),
        "{stderr}"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_writes_a_sorted_deduplicated_input_record_receipt() {
    let temp_dir = tempfile::tempdir().expect("create input receipt temp dir");
    let source = temp_dir.path().join("main.tex");
    let helper = temp_dir.path().join("helper.tex");
    let nested = temp_dir.path().join("nested.tex");
    let receipt = temp_dir.path().join("inputs.tsv");
    let source_bytes = b"\\input helper \\input helper \\end\n";
    let helper_bytes = b"\\input nested \\relax\n";
    let nested_bytes = b"\\relax\n";
    fs::write(&source, source_bytes).expect("write principal input");
    fs::write(&helper, helper_bytes).expect("write included input");
    fs::write(&nested, nested_bytes).expect("write nested input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .arg("--input-records-out")
        .arg(&receipt)
        .output()
        .expect("run input receipt smoke");

    assert!(
        output.status.success(),
        "input receipt run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!(
        "{}\t{}\n{}\t{}\n{}\t{}\n",
        helper_bytes.len(),
        helper.display(),
        source_bytes.len(),
        source.display(),
        nested_bytes.len(),
        nested.display()
    );
    assert_eq!(
        fs::read_to_string(receipt).expect("read input receipt"),
        expected
    );
}
