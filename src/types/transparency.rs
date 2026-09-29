//! Transparency representation for CAD entities

use std::fmt;

/// Transparency source and explicit amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Transparency {
    /// Use the owning layer's transparency.
    ByLayer,
    /// Use the containing block reference's transparency.
    ByBlock,
    /// Explicit transparency amount: 0 is opaque and 255 is transparent.
    Explicit(u8),
}

impl Transparency {
    pub const OPAQUE: Self = Self::Explicit(0);
    pub const TRANSPARENT: Self = Self::Explicit(255);
    pub const BY_LAYER: Self = Self::ByLayer;
    pub const BY_BLOCK: Self = Self::ByBlock;

    pub const fn new(alpha: u8) -> Self {
        Self::Explicit(alpha)
    }

    pub fn from_percent(percent: f64) -> Self {
        // Packed CAD alpha stores opacity rounded down. The complementary
        // transparency amount therefore rounds up to the next byte value.
        Self::Explicit((percent.clamp(0.0, 1.0) * 255.0).ceil() as u8)
    }

    /// Decode a packed DXF or DWG transparency value.
    pub fn from_alpha_value(value: u32) -> Self {
        match (value >> 24) as u8 {
            0 => Self::ByLayer,
            1 => Self::ByBlock,
            2 | 3 => Self::Explicit(255 - (value & 0xFF) as u8),
            _ => Self::ByLayer,
        }
    }

    /// Return the explicit amount, or zero for inherited values.
    pub const fn alpha(&self) -> u8 {
        match self {
            Self::Explicit(alpha) => *alpha,
            Self::ByLayer | Self::ByBlock => 0,
        }
    }

    pub const fn explicit_alpha(&self) -> Option<u8> {
        match self {
            Self::Explicit(alpha) => Some(*alpha),
            Self::ByLayer | Self::ByBlock => None,
        }
    }

    pub fn as_percent(&self) -> f64 {
        self.alpha() as f64 / 255.0
    }

    pub const fn is_by_layer(&self) -> bool {
        matches!(self, Self::ByLayer)
    }

    pub const fn is_by_block(&self) -> bool {
        matches!(self, Self::ByBlock)
    }

    pub const fn is_explicit(&self) -> bool {
        matches!(self, Self::Explicit(_))
    }

    pub const fn is_opaque(&self) -> bool {
        matches!(self, Self::Explicit(0))
    }

    pub const fn is_transparent(&self) -> bool {
        matches!(self, Self::Explicit(255))
    }

    pub const T_10: Self = Self::Explicit(26);
    pub const T_20: Self = Self::Explicit(51);
    pub const T_30: Self = Self::Explicit(77);
    pub const T_40: Self = Self::Explicit(102);
    pub const T_50: Self = Self::Explicit(128);
    pub const T_60: Self = Self::Explicit(153);
    pub const T_70: Self = Self::Explicit(179);
    pub const T_80: Self = Self::Explicit(204);
    pub const T_90: Self = Self::Explicit(230);

    /// Encode the DWG packed form: the same as the DXF one. The high byte is
    /// the method — 0 ByLayer, 1 ByBlock, 2 an explicit amount in the low
    /// byte — in a DWG as in a DXF: AutoCAD and ODA write 0x02 for a value.
    /// A 0x03 read back through ODA as DXF group 440 carries the ByBlock bit,
    /// and ezdxf (with every reader that tests that bit) takes the entity for
    /// opaque. 0x03 is still READ as an explicit amount (`from_alpha_value`).
    pub fn to_alpha_value(&self) -> i32 {
        self.to_dxf_value()
    }

    /// Encode the DXF packed form.
    pub fn to_dxf_value(&self) -> i32 {
        match self {
            Self::ByLayer => 0,
            Self::ByBlock => (1u32 << 24) as i32,
            Self::Explicit(alpha) => ((2u32 << 24) | (255 - *alpha) as u32) as i32,
        }
    }
}

impl Default for Transparency {
    fn default() -> Self {
        Self::ByLayer
    }
}

impl From<u8> for Transparency {
    fn from(alpha: u8) -> Self {
        Self::Explicit(alpha)
    }
}

impl From<Transparency> for u8 {
    fn from(transparency: Transparency) -> Self {
        transparency.alpha()
    }
}

impl fmt::Display for Transparency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByLayer => write!(f, "ByLayer"),
            Self::ByBlock => write!(f, "ByBlock"),
            Self::Explicit(_) => write!(f, "{:.1}%", self.as_percent() * 100.0),
        }
    }
}

#[cfg(feature = "serde")]
pub(crate) const fn opaque() -> Transparency {
    Transparency::OPAQUE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_amounts() {
        let transparency = Transparency::new(128);
        assert_eq!(transparency.explicit_alpha(), Some(128));
        assert_eq!(Transparency::from_percent(0.13).alpha(), 34);
        assert_eq!(Transparency::from_percent(0.30).alpha(), 77);
        assert_eq!(Transparency::from_percent(0.33).alpha(), 85);
        assert_eq!(Transparency::from_percent(0.5).alpha(), 128);
        assert!(Transparency::OPAQUE.is_opaque());
        assert!(Transparency::TRANSPARENT.is_transparent());
    }

    #[test]
    fn packed_methods_roundtrip() {
        for value in [
            Transparency::BY_LAYER,
            Transparency::BY_BLOCK,
            Transparency::OPAQUE,
            Transparency::new(217),
            Transparency::TRANSPARENT,
        ] {
            assert_eq!(
                Transparency::from_alpha_value(value.to_dxf_value() as u32),
                value
            );
            assert_eq!(
                Transparency::from_alpha_value(value.to_alpha_value() as u32),
                value
            );
        }
    }

    /// An explicit amount is written with method 2, in a DWG as in a DXF;
    /// method 3, written here before, still reads as the same amount.
    #[test]
    fn explicit_amounts_are_written_with_method_2() {
        let glass = Transparency::from_alpha_value(0x0200_0059);
        assert_eq!(glass, Transparency::Explicit(255 - 0x59));
        assert_eq!(glass.to_alpha_value() as u32, 0x0200_0059);
        assert_eq!(glass.to_dxf_value() as u32, 0x0200_0059);
        assert_eq!(Transparency::from_alpha_value(0x0300_0059), glass);
        assert_eq!(Transparency::BY_BLOCK.to_alpha_value() as u32, 0x0100_0000);
        assert_eq!(Transparency::BY_LAYER.to_alpha_value(), 0);
        assert_eq!(Transparency::OPAQUE.to_alpha_value() as u32, 0x0200_00FF);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_preserves_method() {
        for value in [
            Transparency::BY_LAYER,
            Transparency::BY_BLOCK,
            Transparency::OPAQUE,
            Transparency::T_50,
        ] {
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<Transparency>(&json).unwrap(), value);
        }
    }
}
