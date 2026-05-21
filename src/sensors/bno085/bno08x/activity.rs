#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum Activity {
    Unknown,
    InVehicle,
    OnBicycle,
    OnFoot,
    Still,
    Tilting,
    Walking,
    Running,
    OnStairs,
}
impl core::fmt::Display for Activity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Activity::Unknown => "Unknown",
            Activity::InVehicle => "In Vehicle",
            Activity::OnBicycle => "On Bicycle",
            Activity::OnFoot => "On Foot",
            Activity::Still => "Still",
            Activity::Tilting => "Tilting",
            Activity::Walking => "Walking",
            Activity::Running => "Running",
            Activity::OnStairs => "On Stairs",
        };
        write!(f, "{}", s)
    }
}

impl From<u8> for Activity {
    fn from(val: u8) -> Self {
        match val {
            1 => Activity::InVehicle,
            2 => Activity::OnBicycle,
            3 => Activity::OnFoot,
            4 => Activity::Still,
            5 => Activity::Tilting,
            6 => Activity::Walking,
            7 => Activity::Running,
            8 => Activity::OnStairs,
            _ => Activity::Unknown,
        }
    }
}
