use crate::sensors::{
    bno085::bno08x::wrapper_async::LastUpdate,
    heading::{Heading, HeadingReading},
};

pub enum ImuCommand {
    EnableRotationVector,
    EnableStepCounter,
    WaitForMotion,
    GetHeading,
    GetStepCount,
}

pub enum ImuReport {
    Heading(HeadingReading),
    StepCount(u16),
    StepDetected,
    ShakeDetected,
    PickupDetected,
    StabilityChanged(bool),
    MotionDetected,
    Activity(super::bno08x::activity::Activity),
    LinearAccel(f32, f32, f32),
    Gyro(f32, f32, f32),
    None,
    Error,
}

impl From<LastUpdate> for ImuReport {
    fn from(update: LastUpdate) -> Self {
        match update {
            LastUpdate::RotationVector(q, acc) => ImuReport::Heading(HeadingReading {
                heading: Heading::from_quaternion(q),
                accuracy_deg: acc,
            }),
            LastUpdate::Activity(a) => ImuReport::Activity(a),
            LastUpdate::StepCount(n) => ImuReport::StepCount(n),
            LastUpdate::StepDetected => ImuReport::StepDetected,
            LastUpdate::ShakeDetected => ImuReport::ShakeDetected,
            LastUpdate::SignificantMotion(detected) => {
                if detected {
                    ImuReport::MotionDetected
                } else {
                    ImuReport::None
                }
            }
            LastUpdate::StabilityDetected(stable) => ImuReport::StabilityChanged(stable),
            LastUpdate::PickupDetected(detected) => {
                if detected {
                    ImuReport::PickupDetected
                } else {
                    ImuReport::None
                }
            }
            LastUpdate::LinearAccel(a) => ImuReport::LinearAccel(a[0], a[1], a[2]), // add ImuReport variant if needed
            LastUpdate::Gyro(g) => ImuReport::Gyro(g[0], g[1], g[2]), // add ImuReport variant if needed
            LastUpdate::None => ImuReport::None,
        }
    }
}
