use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_derive::Deserialize;
use std::fmt;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Limits for running wast tests.
///
/// This is useful for sharing between `tests/wast.rs` and fuzzing, for
/// example, and is used as the minimum threshold for configuration when
/// fuzzing.
///
/// Note that it's ok to increase these numbers if a test comes along and needs
/// it, they're just here as empirically found minimum thresholds so far and
/// they're not too scientific.
pub mod limits {
    pub const MEMORY_SIZE: usize = 805 << 16;
    pub const MEMORIES: u32 = 450;
    pub const TABLES: u32 = 200;
    pub const MEMORIES_PER_MODULE: u32 = 9;
    pub const TABLES_PER_MODULE: u32 = 5;
    pub const COMPONENT_INSTANCES: u32 = 50;
    pub const CORE_INSTANCES: u32 = 900;
    pub const TABLE_ELEMENTS: usize = 1000;
    pub const CORE_INSTANCE_SIZE: usize = 64 * 1024;
    pub const TOTAL_STACKS: u32 = 10;
}

/// Local all `*.wast` tests under `root` which should be the path to the root
/// of the wasmtime repository.
pub fn find_tests(root: &Path) -> Result<Vec<WastTest>> {
    let mut tests = Vec::new();
    add_tests(&mut tests, &root.join("tests/spec_testsuite"), false)?;
    add_tests(&mut tests, &root.join("tests/misc_testsuite"), true)?;
    Ok(tests)
}

fn add_tests(tests: &mut Vec<WastTest>, path: &Path, has_config: bool) -> Result<()> {
    for entry in path.read_dir().context("failed to read directory")? {
        let entry = entry.context("failed to read directory entry")?;
        let path = entry.path();
        if entry
            .file_type()
            .context("failed to get file type")?
            .is_dir()
        {
            add_tests(tests, &path, has_config).context("failed to read sub-directory")?;
            continue;
        }

        if path.extension().and_then(|s| s.to_str()) != Some("wast") {
            continue;
        }

        let contents =
            fs::read_to_string(&path).with_context(|| format!("failed to read test: {path:?}"))?;
        let config = if has_config {
            parse_test_config(&contents, ";;!")
                .with_context(|| format!("failed to parse test configuration: {path:?}"))?
        } else {
            spec_test_config(&path)
        };
        tests.push(WastTest {
            path,
            contents,
            config,
        })
    }
    Ok(())
}

fn spec_test_config(test: &Path) -> TestConfig {
    let mut ret = TestConfig::default();
    ret.spec_test = Some(true);
    match spec_proposal_from_path(test) {
        Some("wide-arithmetic") => {
            ret.wide_arithmetic = Some(true);
        }
        Some("threads") => {
            ret.threads = Some(true);
            ret.reference_types = Some(false);
        }
        Some("relaxed-simd") => {
            ret.relaxed_simd = Some(true);
        }
        Some("custom-page-sizes") => {
            ret.custom_page_sizes = Some(true);
            ret.multi_memory = Some(true);
            ret.memory64 = Some(true);

            // See commentary below in `wasm-3.0` case for why these "hog
            // memory"
            if test.ends_with("memory_max.wast") || test.ends_with("memory_max_i64.wast") {
                ret.hogs_memory = Some(true);
            }
        }
        Some("annotations") => {
            ret.simd = Some(true);
        }
        Some("wasm-3.0") => {
            ret.simd = Some(true);
            ret.relaxed_simd = Some(true);
            ret.multi_memory = Some(true);
            ret.gc = Some(true);
            ret.reference_types = Some(true);
            ret.memory64 = Some(true);
            ret.tail_call = Some(true);
            ret.extended_const = Some(true);

            // Wasmtime, at the current date, has incomplete support for the
            // exceptions proposal. Instead of flagging the entire test suite
            // as needing this proposal try to filter down per-test to what
            // exactly needs this. Other tests aren't expected to need
            // exceptions.
            if test.ends_with("tag.wast")
                || test.ends_with("instance.wast")
                || test.ends_with("throw.wast")
                || test.ends_with("throw_ref.wast")
                || test.ends_with("try_table.wast")
                || test.ends_with("ref_null.wast")
                || test.ends_with("imports.wast")
            {
                ret.exceptions = Some(true);
            }
            if test.parent().unwrap().ends_with("legacy") {
                ret.legacy_exceptions = Some(true);
            }

            // These tests technically don't actually hog any memory but they
            // do have a module definition with a table/memory that is the
            // maximum size. These modules fail to compile in the pooling
            // allocator which has limits on the minimum size of
            // memories/tables by default.
            //
            // Pretend that these hog memory to avoid running the tests in the
            // pooling allocator.
            if test.ends_with("memory.wast")
                || test.ends_with("table.wast")
                || test.ends_with("memory64.wast")
            {
                ret.hogs_memory = Some(true);
            }
        }
        Some(proposal) => panic!("unsuported proposal {proposal:?}"),
        None => {
            ret.reference_types = Some(true);
            ret.simd = Some(true);
        }
    }

    ret
}

/// Parse test configuration from the specified test, comments starting with
/// `;;!`.
pub fn parse_test_config<T>(wat: &str, comment: &'static str) -> Result<T>
where
    T: DeserializeOwned,
{
    // The test config source is the leading lines of the WAT file that are
    // prefixed with `;;!`.
    let config_lines: Vec<_> = wat
        .lines()
        .take_while(|l| l.starts_with(comment))
        .map(|l| &l[comment.len()..])
        .collect();
    let config_text = config_lines.join("\n");

    toml::from_str(&config_text).context("failed to parse the test configuration")
}

/// A `*.wast` test with its path, contents, and configuration.
#[derive(Clone)]
pub struct WastTest {
    pub path: PathBuf,
    pub contents: String,
    pub config: TestConfig,
}

impl fmt::Debug for WastTest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WastTest")
            .field("path", &self.path)
            .field("contents", &"...")
            .field("config", &self.config)
            .finish()
    }
}

macro_rules! foreach_config_option {
    ($m:ident) => {
        $m! {
            memory64
            custom_page_sizes
            multi_memory
            threads
            shared_everything_threads
            gc
            function_references
            relaxed_simd
            reference_types
            tail_call
            extended_const
            wide_arithmetic
            hogs_memory
            nan_canonicalization
            component_model_async
            component_model_async_builtins
            component_model_async_stackful
            component_model_error_context
            component_model_gc
            simd
            gc_types
            exceptions
            legacy_exceptions
            stack_switching
            spec_test
        }
    };
}

macro_rules! define_test_config {
    ($($option:ident)*) => {
        /// Per-test configuration which is written down in the test file itself for
        /// `misc_testsuite/**/*.wast` or in `spec_test_config` above for spec tests.
        #[derive(Debug, PartialEq, Default, Deserialize, Clone)]
        #[serde(deny_unknown_fields)]
        pub struct TestConfig {
            $(pub $option: Option<bool>,)*
        }

        impl TestConfig {
            $(
                pub fn $option(&self) -> bool {
                    self.$option.unwrap_or(false)
                }
            )*
        }
    }
}

foreach_config_option!(define_test_config);

impl TestConfig {
    /// Returns an iterator over each option.
    pub fn options_mut(&mut self) -> impl Iterator<Item = (&'static str, &mut Option<bool>)> {
        macro_rules! mk {
            ($($option:ident)*) => {
                [
                    $((stringify!($option), &mut self.$option),)*
                ].into_iter()
            }
        }
        foreach_config_option!(mk)
    }
}

/// Configuration that spec tests can run under.
#[derive(Debug)]
pub struct WastConfig {
    /// Compiler chosen to run this test.
    pub compiler: Compiler,
    /// Whether or not the pooling allocator is enabled.
    pub pooling: bool,
    /// What garbage collector is being used.
    pub collector: Collector,
}

/// Different compilers that can be tested in Wasmtime.
#[derive(PartialEq, Debug, Copy, Clone)]
pub enum Compiler {
    /// Cranelift backend.
    ///
    /// This tests the Cranelift code generator for native platforms. This
    /// notably excludes Pulley since that's listed separately below even though
    /// Pulley is a backend of Cranelift. This is only used for native code
    /// generation such as x86_64.
    CraneliftNative,

    /// Winch backend.
    ///
    /// This tests the Winch backend for native platforms. Currently Winch
    /// primarily supports x86_64.
    Winch,

    /// Pulley interpreter.
    ///
    /// This tests the Cranelift pulley backend plus the pulley execution
    /// environment of the output bytecode. Note that this is separate from
    /// `Cranelift` above to be able to test both on platforms where Cranelift
    /// has native codegen support.
    CraneliftPulley,
}

impl Compiler {
    /// Returns whether this compiler is known to fail for the provided
    /// `TestConfig`.
    ///
    /// This function will determine if the configuration of the test provided
    /// is known to guarantee fail. This effectively tracks the proposal support
    /// for each compiler backend/runtime and tests whether `config` enables or
    /// disables features that aren't supported.
    ///
    /// Note that this is closely aligned with
    /// `Config::compiler_panicking_wasm_features`.
    pub fn should_fail(&self, config: &TestConfig) -> bool {
        match self {
            Compiler::CraneliftNative => config.legacy_exceptions(),

            Compiler::Winch => {
                let unsupported_base = config.gc()
                    || config.tail_call()
                    || config.function_references()
                    || config.gc()
                    || config.relaxed_simd()
                    || config.gc_types()
                    || config.exceptions()
                    || config.legacy_exceptions()
                    || config.stack_switching()
                    || config.legacy_exceptions();

                if cfg!(target_arch = "x86_64") {
                    return unsupported_base;
                }

                if cfg!(target_arch = "aarch64") {
                    return unsupported_base
                        || config.wide_arithmetic()
                        || (config.simd() && !config.spec_test())
                        || config.threads();
                }

                true
            }

            Compiler::CraneliftPulley => {
                config.threads() || config.legacy_exceptions() || config.stack_switching()
            }
        }
    }

    /// Returns whether this compiler configuration supports the current host
    /// architecture.
    pub fn supports_host(&self) -> bool {
        match self {
            Compiler::CraneliftNative => {
                cfg!(target_arch = "x86_64")
                    || cfg!(target_arch = "aarch64")
                    || cfg!(target_arch = "riscv64")
                    || cfg!(target_arch = "s390x")
            }
            Compiler::Winch => cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64"),
            Compiler::CraneliftPulley => true,
        }
    }
}

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum Collector {
    Auto,
    Null,
    DeferredReferenceCounting,
}

impl WastTest {
    /// Returns whether this test exercises the GC types and might want to use
    /// multiple different garbage collectors.
    pub fn test_uses_gc_types(&self) -> bool {
        self.config.gc() || self.config.function_references()
    }

    /// Returns the optional spec proposal that this test is associated with.
    pub fn spec_proposal(&self) -> Option<&str> {
        spec_proposal_from_path(&self.path)
    }

    /// Returns whether this test should fail under the specified extra
    /// configuration.
    pub fn should_fail(&self, config: &WastConfig) -> bool {
        if !config.compiler.supports_host() {
            return true;
        }

        // Some tests are known to fail with the pooling allocator
        if config.pooling {
            let unsupported = [
                // allocates too much memory for the pooling configuration here
                "misc_testsuite/memory64/more-than-4gb.wast",
                // shared memories + pooling allocator aren't supported yet
                "misc_testsuite/memory-combos.wast",
                "misc_testsuite/threads/LB.wast",
                "misc_testsuite/threads/LB_atomic.wast",
                "misc_testsuite/threads/MP.wast",
                "misc_testsuite/threads/MP_atomic.wast",
                "misc_testsuite/threads/MP_wait.wast",
                "misc_testsuite/threads/SB.wast",
                "misc_testsuite/threads/SB_atomic.wast",
                "misc_testsuite/threads/atomics_notify.wast",
                "misc_testsuite/threads/atomics_wait_address.wast",
                "misc_testsuite/threads/wait_notify.wast",
                "spec_testsuite/proposals/threads/atomic.wast",
                "spec_testsuite/proposals/threads/exports.wast",
                "spec_testsuite/proposals/threads/memory.wast",
            ];

            if unsupported.iter().any(|part| self.path.ends_with(part)) {
                return true;
            }
        }

        if config.compiler.should_fail(&self.config) {
            return true;
        }

        // Disable spec tests per target for proposals that Winch does not implement yet.
        if config.compiler == Compiler::Winch {
            // Common list for tests that fail in all targets supported by Winch.
            let unsupported = [
                "extended-const/elem.wast",
                "extended-const/global.wast",
                "misc_testsuite/component-model/modules.wast",
                "misc_testsuite/externref-id-function.wast",
                "misc_testsuite/externref-segment.wast",
                "misc_testsuite/externref-segments.wast",
                "misc_testsuite/externref-table-dropped-segment-issue-8281.wast",
                "misc_testsuite/linking-errors.wast",
                "misc_testsuite/many_table_gets_lead_to_gc.wast",
                "misc_testsuite/mutable_externref_globals.wast",
                "misc_testsuite/no-mixup-stack-maps.wast",
                "misc_testsuite/no-panic.wast",
                "misc_testsuite/simple_ref_is_null.wast",
                "misc_testsuite/table_grow_with_funcref.wast",
                "spec_testsuite/br_table.wast",
                "spec_testsuite/global.wast",
                "spec_testsuite/ref_func.wast",
                "spec_testsuite/ref_is_null.wast",
                "spec_testsuite/ref_null.wast",
                "spec_testsuite/select.wast",
                "spec_testsuite/table_fill.wast",
                "spec_testsuite/table_get.wast",
                "spec_testsuite/table_grow.wast",
                "spec_testsuite/table_set.wast",
                "spec_testsuite/table_size.wast",
                "spec_testsuite/elem.wast",
                "spec_testsuite/linking.wast",
            ];

            if unsupported.iter().any(|part| self.path.ends_with(part)) {
                return true;
            }

            #[cfg(target_arch = "aarch64")]
            {
                let unsupported = [
                    "misc_testsuite/int-to-float-splat.wast",
                    "misc_testsuite/issue6562.wast",
                    "misc_testsuite/memory64/simd.wast",
                    "misc_testsuite/simd/almost-extmul.wast",
                    "misc_testsuite/simd/canonicalize-nan.wast",
                    "misc_testsuite/simd/cvt-from-uint.wast",
                    "misc_testsuite/simd/edge-of-memory.wast",
                    "misc_testsuite/simd/interesting-float-splat.wast",
                    "misc_testsuite/simd/issue4807.wast",
                    "misc_testsuite/simd/issue6725-no-egraph-panic.wast",
                    "misc_testsuite/simd/issue_3173_select_v128.wast",
                    "misc_testsuite/simd/issue_3327_bnot_lowering.wast",
                    "misc_testsuite/simd/load_splat_out_of_bounds.wast",
                    "misc_testsuite/simd/replace-lane-preserve.wast",
                    "misc_testsuite/simd/spillslot-size-fuzzbug.wast",
                    "misc_testsuite/simd/sse-cannot-fold-unaligned-loads.wast",
                    "misc_testsuite/simd/unaligned-load.wast",
                    "misc_testsuite/simd/v128-select.wast",
                    "misc_testsuite/winch/issue-10331.wast",
                    "misc_testsuite/winch/issue-10357.wast",
                    "misc_testsuite/winch/issue-10460.wast",
                    "misc_testsuite/winch/replace_lane.wast",
                    "misc_testsuite/winch/simd_multivalue.wast",
                    "misc_testsuite/winch/v128_load_lane_invalid_address.wast",
                    "spec_testsuite/proposals/annotations/simd_lane.wast",
                    "spec_testsuite/proposals/multi-memory/simd_memory-multi.wast",
                    "spec_testsuite/simd_address.wast",
                    "spec_testsuite/simd_align.wast",
                    "spec_testsuite/simd_bit_shift.wast",
                    "spec_testsuite/simd_bitwise.wast",
                    "spec_testsuite/simd_boolean.wast",
                    "spec_testsuite/simd_const.wast",
                    "spec_testsuite/simd_conversions.wast",
                    "spec_testsuite/simd_f32x4.wast",
                    "spec_testsuite/simd_f32x4_arith.wast",
                    "spec_testsuite/simd_f32x4_cmp.wast",
                    "spec_testsuite/simd_f32x4_pmin_pmax.wast",
                    "spec_testsuite/simd_f32x4_rounding.wast",
                    "spec_testsuite/simd_f64x2.wast",
                    "spec_testsuite/simd_f64x2_arith.wast",
                    "spec_testsuite/simd_f64x2_cmp.wast",
                    "spec_testsuite/simd_f64x2_pmin_pmax.wast",
                    "spec_testsuite/simd_f64x2_rounding.wast",
                    "spec_testsuite/simd_i16x8_arith.wast",
                    "spec_testsuite/simd_i16x8_arith2.wast",
                    "spec_testsuite/simd_i16x8_cmp.wast",
                    "spec_testsuite/simd_i16x8_extadd_pairwise_i8x16.wast",
                    "spec_testsuite/simd_i16x8_extmul_i8x16.wast",
                    "spec_testsuite/simd_i16x8_q15mulr_sat_s.wast",
                    "spec_testsuite/simd_i16x8_sat_arith.wast",
                    "spec_testsuite/simd_i32x4_arith.wast",
                    "spec_testsuite/simd_i32x4_arith2.wast",
                    "spec_testsuite/simd_i32x4_cmp.wast",
                    "spec_testsuite/simd_i32x4_dot_i16x8.wast",
                    "spec_testsuite/simd_i32x4_extadd_pairwise_i16x8.wast",
                    "spec_testsuite/simd_i32x4_extmul_i16x8.wast",
                    "spec_testsuite/simd_i32x4_trunc_sat_f32x4.wast",
                    "spec_testsuite/simd_i32x4_trunc_sat_f64x2.wast",
                    "spec_testsuite/simd_i64x2_arith.wast",
                    "spec_testsuite/simd_i64x2_arith2.wast",
                    "spec_testsuite/simd_i64x2_cmp.wast",
                    "spec_testsuite/simd_i64x2_extmul_i32x4.wast",
                    "spec_testsuite/simd_i8x16_arith.wast",
                    "spec_testsuite/simd_i8x16_arith2.wast",
                    "spec_testsuite/simd_i8x16_cmp.wast",
                    "spec_testsuite/simd_i8x16_sat_arith.wast",
                    "spec_testsuite/simd_int_to_int_extend.wast",
                    "spec_testsuite/simd_lane.wast",
                    "spec_testsuite/simd_load.wast",
                    "spec_testsuite/simd_load16_lane.wast",
                    "spec_testsuite/simd_load32_lane.wast",
                    "spec_testsuite/simd_load64_lane.wast",
                    "spec_testsuite/simd_load8_lane.wast",
                    "spec_testsuite/simd_load_extend.wast",
                    "spec_testsuite/simd_load_splat.wast",
                    "spec_testsuite/simd_load_zero.wast",
                    "spec_testsuite/simd_select.wast",
                    "spec_testsuite/simd_splat.wast",
                    "spec_testsuite/simd_store.wast",
                    "spec_testsuite/simd_store16_lane.wast",
                    "spec_testsuite/simd_store32_lane.wast",
                    "spec_testsuite/simd_store64_lane.wast",
                    "spec_testsuite/simd_store8_lane.wast",
                ];

                if unsupported.iter().any(|part| self.path.ends_with(part)) {
                    return true;
                }
            }

            #[cfg(target_arch = "x86_64")]
            {
                let unsupported = [
                    // externref/reference-types related
                    // simd-related failures
                    "misc_testsuite/simd/canonicalize-nan.wast",
                ];

                if unsupported.iter().any(|part| self.path.ends_with(part)) {
                    return true;
                }

                // SIMD on Winch requires AVX instructions.
                #[cfg(target_arch = "x86_64")]
                if !(std::is_x86_feature_detected!("avx") && std::is_x86_feature_detected!("avx2"))
                {
                    let unsupported = [
                        "annotations/simd_lane.wast",
                        "memory64/simd.wast",
                        "misc_testsuite/int-to-float-splat.wast",
                        "misc_testsuite/issue6562.wast",
                        "misc_testsuite/simd/almost-extmul.wast",
                        "misc_testsuite/simd/cvt-from-uint.wast",
                        "misc_testsuite/simd/edge-of-memory.wast",
                        "misc_testsuite/simd/issue_3327_bnot_lowering.wast",
                        "misc_testsuite/simd/issue6725-no-egraph-panic.wast",
                        "misc_testsuite/simd/replace-lane-preserve.wast",
                        "misc_testsuite/simd/spillslot-size-fuzzbug.wast",
                        "misc_testsuite/simd/sse-cannot-fold-unaligned-loads.wast",
                        "misc_testsuite/winch/issue-10331.wast",
                        "misc_testsuite/winch/replace_lane.wast",
                        "spec_testsuite/simd_align.wast",
                        "spec_testsuite/simd_boolean.wast",
                        "spec_testsuite/simd_conversions.wast",
                        "spec_testsuite/simd_f32x4.wast",
                        "spec_testsuite/simd_f32x4_arith.wast",
                        "spec_testsuite/simd_f32x4_cmp.wast",
                        "spec_testsuite/simd_f32x4_pmin_pmax.wast",
                        "spec_testsuite/simd_f32x4_rounding.wast",
                        "spec_testsuite/simd_f64x2.wast",
                        "spec_testsuite/simd_f64x2_arith.wast",
                        "spec_testsuite/simd_f64x2_cmp.wast",
                        "spec_testsuite/simd_f64x2_pmin_pmax.wast",
                        "spec_testsuite/simd_f64x2_rounding.wast",
                        "spec_testsuite/simd_i16x8_cmp.wast",
                        "spec_testsuite/simd_i32x4_cmp.wast",
                        "spec_testsuite/simd_i64x2_arith2.wast",
                        "spec_testsuite/simd_i64x2_cmp.wast",
                        "spec_testsuite/simd_i8x16_arith2.wast",
                        "spec_testsuite/simd_i8x16_cmp.wast",
                        "spec_testsuite/simd_int_to_int_extend.wast",
                        "spec_testsuite/simd_load.wast",
                        "spec_testsuite/simd_load_extend.wast",
                        "spec_testsuite/simd_load_splat.wast",
                        "spec_testsuite/simd_load_zero.wast",
                        "spec_testsuite/simd_splat.wast",
                        "spec_testsuite/simd_store16_lane.wast",
                        "spec_testsuite/simd_store32_lane.wast",
                        "spec_testsuite/simd_store64_lane.wast",
                        "spec_testsuite/simd_store8_lane.wast",
                        "spec_testsuite/simd_load16_lane.wast",
                        "spec_testsuite/simd_load32_lane.wast",
                        "spec_testsuite/simd_load64_lane.wast",
                        "spec_testsuite/simd_load8_lane.wast",
                        "spec_testsuite/simd_bitwise.wast",
                        "misc_testsuite/simd/load_splat_out_of_bounds.wast",
                        "misc_testsuite/simd/unaligned-load.wast",
                        "multi-memory/simd_memory-multi.wast",
                        "misc_testsuite/simd/issue4807.wast",
                        "spec_testsuite/simd_const.wast",
                        "spec_testsuite/simd_i8x16_sat_arith.wast",
                        "spec_testsuite/simd_i64x2_arith.wast",
                        "spec_testsuite/simd_i16x8_arith.wast",
                        "spec_testsuite/simd_i16x8_arith2.wast",
                        "spec_testsuite/simd_i16x8_q15mulr_sat_s.wast",
                        "spec_testsuite/simd_i16x8_sat_arith.wast",
                        "spec_testsuite/simd_i32x4_arith.wast",
                        "spec_testsuite/simd_i32x4_dot_i16x8.wast",
                        "spec_testsuite/simd_i32x4_trunc_sat_f32x4.wast",
                        "spec_testsuite/simd_i32x4_trunc_sat_f64x2.wast",
                        "spec_testsuite/simd_i8x16_arith.wast",
                        "spec_testsuite/simd_bit_shift.wast",
                        "spec_testsuite/simd_lane.wast",
                        "spec_testsuite/simd_i16x8_extmul_i8x16.wast",
                        "spec_testsuite/simd_i32x4_extmul_i16x8.wast",
                        "spec_testsuite/simd_i64x2_extmul_i32x4.wast",
                        "spec_testsuite/simd_i16x8_extadd_pairwise_i8x16.wast",
                        "spec_testsuite/simd_i32x4_extadd_pairwise_i16x8.wast",
                        "spec_testsuite/simd_i32x4_arith2.wast",
                    ];

                    if unsupported.iter().any(|part| self.path.ends_with(part)) {
                        return true;
                    }
                }
            }
        }

        // For the exceptions proposal these tests use instructions and such
        // which aren't implemented yet so these are expected to fail.
        if self.config.exceptions() {
            let unsupported = [
                "ref_null.wast",
                "throw.wast",
                "rethrow.wast",
                "throw_ref.wast",
                "try_table.wast",
                "instance.wast",
            ];
            if unsupported.iter().any(|part| self.path.ends_with(part)) {
                return true;
            }
        }

        false
    }
}

fn spec_proposal_from_path(path: &Path) -> Option<&str> {
    let mut iter = path.iter();
    loop {
        match iter.next()?.to_str()? {
            "proposals" => break,
            _ => {}
        }
    }
    Some(iter.next()?.to_str()?)
}
