use gewu_core::AxisScale;

#[uniffi::remote(Enum)]
pub enum AxisScale {
    Linear,
    Log10,
}
