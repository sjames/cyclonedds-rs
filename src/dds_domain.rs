/*
    Copyright 2020 Sojan James

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

//use crate::error::DDSError;
/// Rust wrapper for DDS Domain
///
use cyclonedds_sys::{dds_error::DDSError, DdsDomainId, DdsEntity};
use std::convert::From;
use std::ffi::CString;

#[derive(Debug)]
pub struct DdsDomain(DdsEntity);

impl DdsDomain {
    ///Create a domain with a specified domain id
    pub fn create(domain: DdsDomainId, config: Option<&str>) -> Result<Self, DDSError> {
        unsafe {
            if let Some(cfg) = config {
                let domain_name = CString::new(cfg).expect("Unable to create new config string");
                let d = cyclonedds_sys::dds_create_domain(domain, domain_name.as_ptr());
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

impl From<DdsDomain> for DdsEntity {
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

impl Drop for DdsDomain {
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete domain: {}", ret);
            }
        }
    }
}

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
        let _entity: DdsEntity = DdsEntity::from(dom.unwrap());
    }
}
