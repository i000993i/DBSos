/// Общие трейты для всех драйверов DBSos

#[derive(Debug, Clone, Copy)]
pub enum DeviceType {
    Legacy,
    Pci { vendor: u16, device: u16, class: u8, subclass: u8 },
}

#[derive(Debug, PartialEq)]
pub enum DriverStatus {
    Ok,
    Unsupported,
    Error(&'static str),
}

/// Базовый трейт драйвера
pub trait Driver: Sync {
    fn name(&self) -> &'static str;
    fn device_type(&self) -> DeviceType;
    fn init(&self) -> DriverStatus;
}

pub type DriverEntry = &'static dyn Driver;
