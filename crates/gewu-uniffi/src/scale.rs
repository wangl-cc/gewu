#[derive(uniffi::Enum)]
pub enum AxisScale {
    Linear,
    Log10,
}

impl From<AxisScale> for gewu_core::AxisScale {
    fn from(scale: AxisScale) -> Self {
        match scale {
            AxisScale::Linear => Self::Linear,
            AxisScale::Log10 => Self::Log10,
        }
    }
}

impl From<gewu_core::AxisScale> for AxisScale {
    fn from(scale: gewu_core::AxisScale) -> Self {
        match scale {
            gewu_core::AxisScale::Linear => Self::Linear,
            gewu_core::AxisScale::Log10 => Self::Log10,
        }
    }
}
