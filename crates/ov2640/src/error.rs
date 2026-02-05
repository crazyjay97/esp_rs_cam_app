//!
//! Error from operating the OV2640 Module
//!

#[derive(defmt::Format)]
pub enum OV2640Error<I2CErr> {
    CannotSetImageSizeOnNonJPEG,
    // buffer is too small
    InvalidBufferSize,
    NoI2cPeripheral,
    I2CError(I2CErr),
    NoSpiPeripheral,
}
