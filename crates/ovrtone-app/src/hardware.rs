//! What this computer is: the processor, its cores and vector instructions, the memory, the system,
//! and what the sound devices can do. From that, a class (low, mid, high end) and the buffer size
//! and helper threads that suit it. Nothing here changes a setting by itself: the Performance
//! window shows the suggestion, and a first start with no saved audio setup begins from it.

use ovrtone_engine::DeviceCaps;

/// Rough class of the computer, for the starting buffer size and threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Low,
    Mid,
    High,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low end",
            Self::Mid => "Mid range",
            Self::High => "High end",
        }
    }

    /// A buffer size that plays without gaps on most machines of this class, in frames.
    pub fn buffer(self) -> u32 {
        match self {
            Self::Low => 512,
            Self::Mid => 256,
            Self::High => 128,
        }
    }
}

/// What was found. Fields the system would not tell are `None` or empty.
#[derive(Clone, Debug, Default)]
pub struct Hardware {
    pub cpu: String,
    pub arch: String,
    pub physical: Option<usize>,
    pub logical: usize,
    pub mhz: u64,
    /// The vector instruction sets the processor has (SSE2, AVX2 and so on).
    pub simd: Vec<&'static str>,
    pub ram_gb: f64,
    pub free_gb: f64,
    pub os: String,
    /// The output devices of the usual audio system and what each can do (not ASIO drivers).
    pub devices: Vec<DeviceCaps>,
}

/// The vector instruction sets of this processor.
pub fn simd_sets() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut found = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        macro_rules! check {
            ($($f:tt => $name:expr),*) => {
                $(if std::is_x86_feature_detected!($f) { found.push($name); })*
            };
        }
        check!(
            "sse2" => "SSE2",
            "sse3" => "SSE3",
            "ssse3" => "SSSE3",
            "sse4.1" => "SSE4.1",
            "sse4.2" => "SSE4.2",
            "avx" => "AVX",
            "avx2" => "AVX2",
            "fma" => "FMA",
            "avx512f" => "AVX-512"
        );
    }
    #[cfg(target_arch = "aarch64")]
    found.push("NEON");
    found
}

/// Physical cores, as the system says.
pub fn physical_cores() -> Option<usize> {
    sysinfo::System::physical_core_count()
}

/// Everything, devices included. Some drivers are slow to answer: call it off the UI thread.
pub fn detect() -> Hardware {
    let mut hw = quick();
    hw.devices = ovrtone_engine::Engine::device_caps("");
    hw
}

/// The processor, memory and system, without asking the sound devices. Fast.
pub fn quick() -> Hardware {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    let gb = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
    let (cpu, mhz) = sys
        .cpus()
        .first()
        .map_or((String::new(), 0), |c| (c.brand().trim().to_string(), c.frequency()));
    Hardware {
        cpu: if cpu.is_empty() {
            "Unknown processor".to_string()
        } else {
            cpu
        },
        arch: sysinfo::System::cpu_arch(),
        physical: physical_cores(),
        logical: std::thread::available_parallelism().map_or(1, |n| n.get()),
        mhz,
        simd: simd_sets(),
        ram_gb: gb(sys.total_memory()),
        free_gb: gb(sys.available_memory()),
        os: sysinfo::System::long_os_version().unwrap_or_default(),
        devices: Vec::new(),
    }
}

impl Hardware {
    pub fn cores(&self) -> usize {
        self.physical.unwrap_or(self.logical).max(1)
    }

    /// The class of this computer. When the system said little, the middle one: its buffer is
    /// safe on most machines.
    pub fn tier(&self) -> Tier {
        let cores = self.cores();
        let avx2 = self.simd.contains(&"AVX2");
        if self.ram_gb <= 0.0 {
            return Tier::Mid;
        }
        if cores <= 2 || self.ram_gb < 7.5 {
            Tier::Low
        } else if cores >= 8 && self.ram_gb >= 15.5 && (avx2 || !cfg!(target_arch = "x86_64")) {
            Tier::High
        } else {
            Tier::Mid
        }
    }

    /// The buffer size to start from: the class's, within what the device takes.
    pub fn suggested_buffer(&self, device: Option<&DeviceCaps>) -> u32 {
        let want = self.tier().buffer();
        match device.and_then(|d| d.buffer) {
            Some((lo, hi)) if lo <= hi => want.clamp(lo, hi),
            _ => want,
        }
    }

    /// Helper threads for plugins and strips: half the physical cores, at least one.
    pub fn suggested_workers(&self) -> usize {
        ovrtone_engine::parallel::default_workers(self.physical)
    }

    /// The device playback goes to: the one named, else the system's default.
    pub fn device(&self, name: &str) -> Option<&DeviceCaps> {
        self.devices
            .iter()
            .find(|d| !name.is_empty() && d.name == name)
            .or_else(|| self.devices.iter().find(|d| d.is_default))
    }

    /// Plain lines for a report: no names of the user's, only the machine.
    pub fn report(&self) -> String {
        use std::fmt::Write as _;
        let mut r = String::new();
        let _ = writeln!(r, "cpu={}", self.cpu);
        let _ = writeln!(r, "arch={}", self.arch);
        let _ = writeln!(
            r,
            "cores_physical={}",
            self.physical.map_or("unknown".to_string(), |n| n.to_string())
        );
        let _ = writeln!(r, "cores_logical={}", self.logical);
        let _ = writeln!(r, "cpu_mhz={}", self.mhz);
        let _ = writeln!(r, "simd={}", self.simd.join(" "));
        let _ = writeln!(r, "engine_simd={}", ovrtone_engine::simd_level().name());
        let _ = writeln!(r, "ram_gb={:.1}", self.ram_gb);
        let _ = writeln!(r, "os={}", self.os);
        let _ = writeln!(r, "tier={}", self.tier().name());
        for d in &self.devices {
            let _ = writeln!(
                r,
                "device={} default={} channels={} rates={:?} buffer={:?}",
                d.name, d.is_default, d.channels, d.rates, d.buffer
            );
        }
        r
    }
}
