use std::{error::Error, fmt};

use cyclonedds_sys::{dds_entity_t};

#[derive(Debug,PartialEq)]
pub enum DDSError {
    DdsOk,
    DdsError,
    Unsupported,
    BadParameter,
    PreconditionNotMet,
    OutOfResources,
    NotEnabled,
    ImmutablePolicy,
    InconsistentPolicy,
    AlreadyDeleted,
    Timeout,
    NoData,
    IllegalOperation,
    NotAllowedBySecurity,
}

impl Error for DDSError {}

impl fmt::Display for DDSError {
    fn fmt(&self, f:&mut fmt::Formatter) -> fmt::Result {
        match self {
            DDSError::DdsOk => write!(f,"OK"),
            DDSError::DdsError => write!(f,"Unspecified Error"),
            DDSError::Unsupported => write!(f,"Unsupported"),
            DDSError::BadParameter => write!(f,"Bad parameter"),
            DDSError::PreconditionNotMet => write!(f,"Preconditions not met"),
            DDSError::OutOfResources => write!(f,"Out of resources"),
            DDSError::NotEnabled => write!(f,"Not enabled"),
            DDSError::ImmutablePolicy => write!(f,"Immutable policy"),
            DDSError::InconsistentPolicy => write!(f,"Inconsistent polocy"),
            DDSError::AlreadyDeleted => write!(f,"Already deleted"),
            DDSError::Timeout => write!(f,"Timeout"),
            DDSError::NoData => write!(f,"No Data"),
            DDSError::IllegalOperation => write!(f,"Illegal operation"),
            DDSError::NotAllowedBySecurity => write!(f,"Not allowed by security"),
        }
    }
}

/// These constants are defined in ddsrt/retcode.h. bindgen doesn't see these macros
/// and hence they are redefined here.DDSError
/// Bad things will happen if these go out of sync
impl From<dds_entity_t> for DDSError {
    fn from(entity : dds_entity_t) -> Self {
        match Some(entity) {
            Some(0) => DDSError::DdsOk,
            Some(-1) => DDSError::DdsError,
            Some(-2) => DDSError::Unsupported,
            Some(-3) => DDSError::BadParameter,
            Some(-4) => DDSError::PreconditionNotMet,
            Some(-5) => DDSError::OutOfResources,
            Some(-6) => DDSError::NotEnabled,
            Some(-7) => DDSError::ImmutablePolicy,
            Some(-8) => DDSError::InconsistentPolicy,
            Some(-9) => DDSError::AlreadyDeleted,
            Some(-10) => DDSError::Timeout,
            Some(-11) => DDSError::NoData,
            Some(-12) => DDSError::IllegalOperation,
            Some(-13) => DDSError::NotAllowedBySecurity,
            Some(x) if x > 0 => DDSError::DdsOk,
            Some(x) if x < -13 => DDSError::DdsError,
            None => DDSError::DdsError,
            _ => DDSError::DdsError,
        }
    }
}
