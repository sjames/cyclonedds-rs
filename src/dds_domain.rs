use crate::error::DDSError;
/// Rust wrapper for DDS Domain
///
use cyclonedds_sys::{dds_entity_t, *};
use std::convert::From;
use std::ffi::CString;

#[derive(Debug)]
pub struct DdsDomain(dds_entity_t);

impl DdsDomain {
    ///Create a domain with a specified domain id
    pub fn create(domain: dds_domainid_t, config: Option<&str>) -> Result<Self, DDSError> {
        unsafe {
            if let Some(cfg) = config {
                let d = cyclonedds_sys::dds_create_domain(
                    domain,
                    CString::new(cfg)
                        .expect("Unable to create new config string")
                        .as_ptr(),
                );
                // negative return value signify an error
                if d > 0 {
                    Ok(DdsDomain(d))
                } else {
                    Err(DDSError::from(d))
                }
            } else {
                let d = cyclonedds_sys::dds_create_domain(domain, std::ptr::null());

                if d > 0 {
                    Ok(DdsDomain(d))
                } else {
                    Err(DDSError::from(d))
                }
            }
        }
    }
}

impl From<DdsDomain> for dds_entity_t {
    fn from(domain: DdsDomain) -> Self {
        domain.0
    }
}

impl PartialEq for DdsDomain {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for DdsDomain {}

#[cfg(test)]
mod dds_domain_tests {
    use super::*;

    #[test]
    fn test_create_domain_with_bad_config() {
        assert_ne!(Err(DDSError::DdsOk), DdsDomain::create(0, Some("blah")));
    }

    #[test]
    fn test_create_domain_with_id() {
        let dom = DdsDomain::create(10, None);
        let entity: dds_entity_t = dds_entity_t::from(dom.unwrap());
    }

}
