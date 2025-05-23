use crate::helpers::{avg_vecu32, nvd_r2u64, pop_4u8};
use bincode::Encode;
use serde::Serialize;
use sysinfo::MemoryRefreshKind;
use tokio::io::AsyncReadExt;

/*
typedef struct {
    uint8_t cpu_usage;
    uint16_t ram_max;
    uint8_t ram_usage;
    char ram_unit[4];
    uint8_t gpu_usage;
    uint16_t vram_max;
    uint8_t vram_usage;
    char vram_unit[4];
} DataStruct;
*/
const CREATE_NO_WINDOW: u32 = 0x08000000;
const CPU_TJMAX: u8 = 89;
const GPU_MAX_TEMP: u8 = 88;

#[derive(Serialize, Debug, Clone, Encode)]
pub struct SystemInfo {
    pub cpu_usage: u8,
    pub ram_max: u16,
    pub ram_usage: u8,
    pub ram_unit: [u8; 4],
    pub gpu_usage: u8,
    pub vram_max: u16,
    pub vram_usage: u8,
    pub vram_unit: [u8; 4],
    pub cpu_temp: u8,
    pub gpu_temp: u8,
    pub cpu_tjmax: u8,
    pub gpu_max_temp: u8,
}

impl SystemInfo {
    fn get_unit(exp: u32) -> String {
        match exp {
            0 => "B",
            1 => "KB",
            2 => "MB",
            3 => "GB",
            4 => "TB",
            _ => "UB",
        }
        .to_owned()
    }

    fn get_exp(num: u64, base: u64) -> u32 {
        match num {
            x if x > u64::pow(base, 4) => 4,
            x if x > u64::pow(base, 3) => 3,
            x if x > u64::pow(base, 2) => 2,
            x if x > base => 1,
            _ => 0,
        }
    }

    pub async fn get_system_info(system_info: &mut sysinfo::System) -> Self {
        // Need to refresh only CPU and RAM => big boost when combined with reusing system_info
        // system_info.refresh_all();
        system_info.refresh_memory_specifics(MemoryRefreshKind::everything().with_ram());
        let base = 1024;

        let ram_max = system_info.total_memory();
        let ram_exp = Self::get_exp(ram_max, base);

        let gpu_info = GpuInfo::get_gpu_info().await;
        let vram_mult = u64::pow(base, 2); // MiB

        let vram_max = match &gpu_info {
            Some(gi) => gi.vram_max * vram_mult,
            None => 0,
        };
        let vram_exp = Self::get_exp(vram_max, base);

        // Refresh only CPU usage before reading
        system_info.refresh_cpu_usage();
        SystemInfo {
            cpu_usage: avg_vecu32(
                system_info
                    .cpus()
                    .iter()
                    .map(|c| c.cpu_usage() as u32)
                    .collect(),
            ) as u8,
            cpu_temp: 0,
            // cpu_usage: system_info.cpus().first().unwrap().cpu_usage() as u8,
            ram_max: (ram_max as f64 / u64::pow(base, ram_exp) as f64 * 10.0) as u16,
            ram_usage: (system_info.used_memory() as f64 / ram_max as f64 * 100.0) as u8,
            ram_unit: pop_4u8(Self::get_unit(ram_exp).as_bytes()),
            gpu_usage: match &gpu_info {
                Some(gi) => gi.gpu_usage as u8,
                None => u8::MAX,
            },
            vram_max: (vram_max as f64 / u64::pow(base, vram_exp) as f64 * 10.0) as u16,
            vram_usage: match &gpu_info {
                Some(gi) => {
                    (gi.vram_used as f64 * vram_mult as f64 / vram_max as f64 * 100.0) as u8
                }
                None => u8::MAX,
            },
            vram_unit: pop_4u8(Self::get_unit(vram_exp).as_bytes()),
            gpu_temp: match &gpu_info {
                Some(gi) => gi.gpu_temp as u8,
                None => u8::MAX,
            },
            cpu_tjmax: CPU_TJMAX,
            gpu_max_temp: GPU_MAX_TEMP,
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct GpuInfo {
    pub gpu_usage: u64,
    pub vram_max: u64,
    pub vram_used: u64,
    pub gpu_temp: u64,
}

impl GpuInfo {
    pub async fn get_gpu_info() -> Option<Self> {
        // TODO: AMD support
        let Ok(mut cmd) = tokio::process::Command::new("nvidia-smi")
            .arg("-q")
            .arg("-x")
            .stdout(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        else {
            return None;
        };

        let stdout = cmd.stdout.take().unwrap();
        let mut stdout_reader = tokio::io::BufReader::new(stdout);
        let mut mut_stdout = String::new();
        if stdout_reader.read_to_string(&mut mut_stdout).await.is_err() {
            return None;
        };

        match xmltojson::to_json(&mut_stdout) {
            Ok(json) => {
                let g = json["nvidia_smi_log"]["gpu"].to_owned();

                let Some(gpu_usage) = nvd_r2u64(g["utilization"]["gpu_util"].to_string()) else {
                    return None;
                };
                let Some(vram_max) = nvd_r2u64(g["fb_memory_usage"]["total"].to_string()) else {
                    return None;
                };
                let Some(vram_used) = nvd_r2u64(g["fb_memory_usage"]["used"].to_string()) else {
                    return None;
                };
                let Some(gpu_temp) = nvd_r2u64(g["temperature"]["gpu_temp"].to_string()) else {
                    return None;
                };

                Some(GpuInfo {
                    gpu_usage,
                    vram_max,
                    vram_used,
                    gpu_temp,
                })
            }
            Err(_) => None,
        }
    }
}
