//! GPU makers, known by their adapters' names: the label a GPU gets in the meters (its maker's stock ticker), the
//! color its bars and memory pie light up in, and the color it glows in. The TAP colors the engines meter with it,
//! the daemon the memory pies; it touches nothing of XAML.

/// A GPU's maker.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Vendor {
    pub ticker: &'static str,
    /// A shorter label, for `vendor_label = "short"`: NV, RAD (Radeon, as Mesa's RADV has it); the rest as their
    /// tickers.
    pub short: &'static str,
    /// RGB.
    pub bar: [u8; 3],
    pub glow: [u8; 3],
}

/// Brand colors: NVIDIA green (NVIDIA's own brand page), Radeon red (AMD's red), Intel's bright cyan for the bars
/// with its classic blue for the glow, Snapdragon gold with Snapdragon red glowing (so Qualcomm and Radeon don't
/// read alike), and Microsoft blue for Basic Render Driver.
pub fn vendor(adapter: &str) -> Option<Vendor> {
    let a = adapter.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|k| a.contains(k));
    Some(if has(&["nvidia", "geforce", "quadro"]) {
        Vendor { ticker: "NVDA", short: "NV", bar: [0x76, 0xB9, 0x00], glow: [0x76, 0xB9, 0x00] }
    } else if has(&["amd", "radeon", "ati "]) {
        Vendor { ticker: "AMD", short: "RAD", bar: [0xED, 0x1C, 0x24], glow: [0xED, 0x1C, 0x24] }
    } else if has(&["intel", "arc(tm)", "iris"]) {
        Vendor { ticker: "INTC", short: "INTC", bar: [0x00, 0xC7, 0xFD], glow: [0x00, 0x68, 0xB5] }
    } else if has(&["qualcomm", "adreno", "snapdragon"]) {
        Vendor { ticker: "QCOM", short: "QCOM", bar: [0xD9, 0xA8, 0x2E], glow: [0xE5, 0x19, 0x37] }
    } else if has(&["microsoft basic render", "microsoft basic display"]) {
        Vendor { ticker: "MSFT", short: "MSFT", bar: [0x00, 0xA4, 0xEF], glow: [0x00, 0xA4, 0xEF] }
    } else {
        return None;
    })
}

impl Vendor {
    /// The bar color as `#RRGGBB`.
    pub fn bar_hex(&self) -> String {
        let [r, g, b] = self.bar;
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makers() {
        assert_eq!(vendor("NVIDIA GeForce RTX 4090").map(|v| v.ticker), Some("NVDA"));
        assert_eq!(vendor("AMD Radeon(TM) Graphics").map(|v| v.ticker), Some("AMD"));
        assert_eq!(vendor("Intel(R) UHD Graphics 770").map(|v| v.ticker), Some("INTC"));
        assert_eq!(vendor("Qualcomm(R) Adreno(TM) X1-85 GPU").map(|v| v.ticker), Some("QCOM"));
        assert_eq!(vendor("Microsoft Basic Render Driver").map(|v| v.ticker), Some("MSFT"));
        assert_eq!(vendor("Moore Threads MTT S80"), None);
    }
}
