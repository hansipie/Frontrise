//! Which GStreamer video decoders Frontrise may use.
//!
//! On hybrid laptops (AMD or Intel iGPU drives the screen, NVIDIA dGPU on the
//! side) NVDEC and VA-API decoders share the same rank, so GStreamer picks
//! NVDEC. That wakes the dGPU on every video load and sends every frame
//! through cross-GPU interop to GTK, which renders on the other GPU.
//! NVDEC is therefore only allowed when NVIDIA drives the display.

use std::path::Path;

const NVIDIA: u32 = 0x10de;

/// Decoders disabled whenever NVDEC is not wanted.
pub const NVIDIA_DECODERS_OFF: &str = "nvh264dec:0,nvh265dec:0,nvvp9dec:0,nvav1dec:0,\
    nvh264sldec:0,nvh265sldec:0,nvautogpuh264dec:0,nvautogpuh265dec:0,\
    nvautogpuvp9dec:0,nvautogpuav1dec:0";

/// A GPU seen in `/sys/class/drm`: PCI vendor id, and whether the firmware
/// used it for the boot display (`None` when the kernel does not say).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gpu {
    pub vendor: u32,
    pub boot_vga: Option<bool>,
}

/// True if the NVIDIA GPU is the one that drives the display: it is the boot
/// display, or it is the only kind of GPU there is.
pub fn nvidia_drives_display(gpus: &[Gpu]) -> bool {
    if let Some(boot) = gpus.iter().find(|g| g.boot_vga == Some(true)) {
        return boot.vendor == NVIDIA;
    }
    !gpus.is_empty() && gpus.iter().all(|g| g.vendor == NVIDIA)
}

/// NVDEC is used only if the user asked for hardware decoding and NVIDIA
/// drives the display. Otherwise hardware decoding goes through VA-API on the
/// display GPU, or libav in software.
pub fn allow_nvidia_decoders(hardware_decoding: bool, gpus: &[Gpu]) -> bool {
    hardware_decoding && nvidia_drives_display(gpus)
}

/// The GPUs of this machine.
pub fn gpus() -> Vec<Gpu> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return vec![] };
    let read = |p: &Path| std::fs::read_to_string(p).ok().map(|s| s.trim().to_owned());
    let mut gpus = vec![];
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // card0, card1… (not card0-eDP-1 connectors, not renderD*).
        if !name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let device = entry.path().join("device");
        let Some(vendor) = read(&device.join("vendor"))
            .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        else {
            continue;
        };
        let boot_vga = read(&device.join("boot_vga")).map(|v| v == "1");
        gpus.push(Gpu { vendor, boot_vga });
    }
    gpus
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMD: u32 = 0x1002;
    const INTEL: u32 = 0x8086;

    #[test]
    fn hybrid_laptop_with_igpu_display_keeps_nvdec_off() {
        // The iGPU drives the screen: decode there, keep the dGPU asleep.
        let gpus = [
            Gpu { vendor: AMD, boot_vga: Some(true) },
            Gpu { vendor: NVIDIA, boot_vga: Some(false) },
        ];
        assert!(!allow_nvidia_decoders(true, &gpus));
        let gpus = [
            Gpu { vendor: NVIDIA, boot_vga: Some(false) },
            Gpu { vendor: INTEL, boot_vga: Some(true) },
        ];
        assert!(!allow_nvidia_decoders(true, &gpus));
    }

    #[test]
    fn nvidia_display_allows_nvdec_when_asked() {
        let dgpu_mode = [
            Gpu { vendor: AMD, boot_vga: Some(false) },
            Gpu { vendor: NVIDIA, boot_vga: Some(true) },
        ];
        assert!(allow_nvidia_decoders(true, &dgpu_mode));
        assert!(!allow_nvidia_decoders(false, &dgpu_mode));
        let desktop = [Gpu { vendor: NVIDIA, boot_vga: None }];
        assert!(allow_nvidia_decoders(true, &desktop));
    }

    #[test]
    fn unknown_layout_keeps_nvdec_off() {
        assert!(!allow_nvidia_decoders(true, &[]));
        let mixed = [Gpu { vendor: AMD, boot_vga: None }, Gpu { vendor: NVIDIA, boot_vga: None }];
        assert!(!allow_nvidia_decoders(true, &mixed));
    }
}
