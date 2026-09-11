//! Read-once hardware guidance for the transcription model pickers.
//!
//! This is deliberately separate from `ModelRuntimeManager`: guidance never
//! selects, installs, loads, unloads, or hides a model.

use serde::Serialize;
use std::sync::OnceLock;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChipTier {
    LegacyAppleSilicon,
    CurrentAppleSilicon,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelHardwareGuidance {
    pub schema_version: u8,
    pub chip: Option<String>,
    pub chip_tier: ChipTier,
    pub physical_memory_gib: Option<u64>,
    pub model_memory_budget_mib: u64,
    pub recommended_model: String,
    pub warned_models: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModelMemoryCost {
    model_name: &'static str,
    catalog_size: &'static str,
    working_memory_mib: u64,
}

// A presentation-only budget table keyed to every shipped catalog entry. The
// estimates include model weights plus ordinary decoder/runtime headroom; they
// are never consumed by the download or runtime lifecycle.
const MODEL_MEMORY_COSTS: &[ModelMemoryCost] = &[
    ModelMemoryCost {
        model_name: "parakeet-tdt-0.6b-v3-coreml",
        catalog_size: "~470 MB",
        working_memory_mib: 1_536,
    },
    ModelMemoryCost {
        model_name: "parakeet-tdt-0.6b-v2-fp16",
        catalog_size: "~1.2 GB",
        working_memory_mib: 3_072,
    },
    ModelMemoryCost {
        model_name: "tiny.en",
        catalog_size: "~75 MB",
        working_memory_mib: 384,
    },
    ModelMemoryCost {
        model_name: "base.en",
        catalog_size: "~150 MB",
        working_memory_mib: 768,
    },
    ModelMemoryCost {
        model_name: "small.en",
        catalog_size: "~500 MB",
        working_memory_mib: 1_536,
    },
    ModelMemoryCost {
        model_name: "medium.en",
        catalog_size: "~1.5 GB",
        working_memory_mib: 4_096,
    },
    ModelMemoryCost {
        model_name: "large-v3-turbo",
        catalog_size: "~3 GB",
        working_memory_mib: 7_168,
    },
];

static GUIDANCE: OnceLock<ModelHardwareGuidance> = OnceLock::new();

pub fn get() -> ModelHardwareGuidance {
    GUIDANCE.get_or_init(detect).clone()
}

fn detect() -> ModelHardwareGuidance {
    #[cfg(target_os = "macos")]
    {
        let chip = sysctl_string(c"machdep.cpu.brand_string")
            .map(|value| value.chars().take(64).collect());
        let physical_memory_gib = sysctl_u64(c"hw.memsize").map(|bytes| bytes / GIB);
        guidance_for(chip, physical_memory_gib)
    }

    #[cfg(not(target_os = "macos"))]
    {
        guidance_for(None, None)
    }
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut length = 0usize;
    // SAFETY: the first call supplies a valid NUL-terminated name and asks
    // macOS only for the required output length.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || length == 0
        || length > 256
    {
        return None;
    }
    let mut bytes = vec![0u8; length];
    // SAFETY: `bytes` has exactly the capacity reported by the first call and
    // remains alive and exclusively borrowed for the write.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    bytes.truncate(length);
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let value = String::from_utf8(bytes).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &std::ffi::CStr) -> Option<u64> {
    let mut value = 0u64;
    let mut length = std::mem::size_of::<u64>();
    // SAFETY: `value` is an aligned writable `u64`, and `length` describes its
    // exact storage. Both outlive the call.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && length == std::mem::size_of::<u64>()).then_some(value)
}

fn chip_tier(chip: Option<&str>) -> ChipTier {
    let Some(generation) = chip
        .and_then(|value| value.strip_prefix("Apple M"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u16>().ok())
    else {
        return ChipTier::Unknown;
    };
    if generation <= 2 {
        ChipTier::LegacyAppleSilicon
    } else {
        ChipTier::CurrentAppleSilicon
    }
}

fn memory_budget_mib(tier: ChipTier, physical_memory_gib: Option<u64>) -> u64 {
    match physical_memory_gib {
        Some(memory) if memory >= 32 => 8_192,
        Some(memory) if memory >= 24 => 7_168,
        Some(memory) if memory >= 16 => match tier {
            ChipTier::LegacyAppleSilicon => 3_072,
            ChipTier::CurrentAppleSilicon | ChipTier::Unknown => 4_096,
        },
        Some(memory) if memory >= 12 => 3_072,
        Some(_) => 1_536,
        None => 3_072,
    }
}

fn guidance_for(chip: Option<String>, physical_memory_gib: Option<u64>) -> ModelHardwareGuidance {
    debug_assert!(MODEL_MEMORY_COSTS.iter().all(|entry| {
        crate::model_runtime::model_definition(entry.model_name)
            .is_ok_and(|definition| definition.size == entry.catalog_size)
    }));
    let tier = chip_tier(chip.as_deref());
    let budget = memory_budget_mib(tier, physical_memory_gib);
    let warned_models = MODEL_MEMORY_COSTS
        .iter()
        .filter(|model| model.working_memory_mib > budget)
        .map(|model| model.model_name.to_string())
        .collect();
    let recommended_model = if tier == ChipTier::Unknown {
        "base.en"
    } else {
        "parakeet-tdt-0.6b-v3-coreml"
    };

    ModelHardwareGuidance {
        schema_version: 1,
        chip,
        chip_tier: tier,
        physical_memory_gib,
        model_memory_budget_mib: budget,
        recommended_model: recommended_model.to_string(),
        warned_models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn budget_table_matches_every_catalog_model_and_size() {
        let guidance: BTreeMap<_, _> = MODEL_MEMORY_COSTS
            .iter()
            .map(|model| (model.model_name, model.catalog_size))
            .collect();
        let catalog: BTreeMap<_, _> = crate::model_runtime::MODEL_DEFINITIONS
            .iter()
            .map(|model| (model.model_name, model.size))
            .collect();
        assert_eq!(guidance, catalog);
    }

    #[test]
    fn low_ram_apple_silicon_warns_only_models_above_budget() {
        let guidance = guidance_for(Some("Apple M1".to_string()), Some(8));
        assert_eq!(guidance.chip_tier, ChipTier::LegacyAppleSilicon);
        assert_eq!(guidance.model_memory_budget_mib, 1_536);
        assert_eq!(
            guidance.warned_models,
            vec!["parakeet-tdt-0.6b-v2-fp16", "medium.en", "large-v3-turbo"]
        );
        assert_eq!(guidance.recommended_model, "parakeet-tdt-0.6b-v3-coreml");
    }

    #[test]
    fn older_chip_gets_a_more_conservative_sixteen_gib_budget() {
        let legacy = guidance_for(Some("Apple M2 Pro".to_string()), Some(16));
        let current = guidance_for(Some("Apple M5 Pro".to_string()), Some(16));
        assert!(legacy.warned_models.contains(&"medium.en".to_string()));
        assert!(!current.warned_models.contains(&"medium.en".to_string()));
        assert!(current
            .warned_models
            .contains(&"large-v3-turbo".to_string()));
    }

    #[test]
    fn high_spec_current_apple_silicon_keeps_default_and_has_no_warnings() {
        let guidance = guidance_for(Some("Apple M5 Pro".to_string()), Some(48));
        assert_eq!(guidance.chip_tier, ChipTier::CurrentAppleSilicon);
        assert_eq!(guidance.recommended_model, "parakeet-tdt-0.6b-v3-coreml");
        assert!(guidance.warned_models.is_empty());
    }
}
