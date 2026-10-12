//! Choice of the graphics backend. It must be known before the window opens, so it is stored in
//! a small file of its own instead of the regular (window-dependent) settings.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// OpenGL – works almost everywhere, including VMs and older GPUs.
    Glow,
    /// wgpu – Vulkan/Metal/DirectX 12.
    Wgpu,
}

impl Renderer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Glow => "OpenGL",
            Self::Wgpu => "wgpu (Vulkan/Metal/DX12)",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "glow" | "opengl" | "gl" => Some(Self::Glow),
            "wgpu" => Some(Self::Wgpu),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Glow => "glow",
            Self::Wgpu => "wgpu",
        }
    }

    pub fn to_eframe(self) -> eframe::Renderer {
        match self {
            Self::Glow => eframe::Renderer::Glow,
            Self::Wgpu => eframe::Renderer::Wgpu,
        }
    }

    fn file() -> Option<PathBuf> {
        directories::ProjectDirs::from("org", "shadowice", "andOTP").map(|d| d.config_dir().join("renderer"))
    }

    /// `ANDOTP_RENDERER` (glow/wgpu), then the saved choice, then OpenGL.
    pub fn load() -> Self {
        std::env::var("ANDOTP_RENDERER")
            .ok()
            .and_then(|v| Self::parse(&v))
            .or_else(|| Self::file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| Self::parse(&s)))
            .unwrap_or(Self::Glow)
    }

    pub fn save(self) -> std::io::Result<()> {
        let Some(file) = Self::file() else { return Ok(()) };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(Renderer::parse("OpenGL"), Some(Renderer::Glow));
        assert_eq!(Renderer::parse(" wgpu\n"), Some(Renderer::Wgpu));
        assert_eq!(Renderer::parse("vulkan"), None);
        for r in [Renderer::Glow, Renderer::Wgpu] {
            assert_eq!(Renderer::parse(r.as_str()), Some(r));
        }
    }
}
