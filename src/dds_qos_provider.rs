/*
    Copyright 2026 Sojan James

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

use cyclonedds_sys::*;
use std::ffi::CString;

pub use cyclonedds_sys::dds_qos_kind;

use crate::dds_qos::DdsQos;

/// Loads QoS profiles from an XML system-definition file at runtime (a "USER_QOS_PROFILES"
/// style document, in the sense ROS2 or OpenDDS users would recognize it), rather than
/// building DdsQos values in code.
pub struct DdsQosProvider(*mut dds_qos_provider_t);

unsafe impl Send for DdsQosProvider {}

impl DdsQosProvider {
    /// `path` is either a filesystem path to the definition file, or the XML content itself.
    pub fn create(path: &str) -> Result<Self, DDSError> {
        let path = CString::new(path).map_err(|_| DDSError::BadParameter)?;
        let mut provider: *mut dds_qos_provider_t = std::ptr::null_mut();
        let ret = unsafe { dds_create_qos_provider(path.as_ptr(), &mut provider) };
        if ret == 0 {
            Ok(DdsQosProvider(provider))
        } else {
            Err(DDSError::from(ret))
        }
    }

    /// Same as create(), but only the QoS profiles matching `key` (in
    /// "<library>::<profile>::<entity>" format, wildcards allowed) are loaded.
    pub fn create_scoped(path: &str, key: &str) -> Result<Self, DDSError> {
        let path = CString::new(path).map_err(|_| DDSError::BadParameter)?;
        let key = CString::new(key).map_err(|_| DDSError::BadParameter)?;
        let mut provider: *mut dds_qos_provider_t = std::ptr::null_mut();
        let ret =
            unsafe { dds_create_qos_provider_scope(path.as_ptr(), &mut provider, key.as_ptr()) };
        if ret == 0 {
            Ok(DdsQosProvider(provider))
        } else {
            Err(DDSError::from(ret))
        }
    }

    /// Look up a QoS profile by its full "<library>::<profile>::<entity>" key and entity
    /// kind. The dds_qos_t cyclone returns here is owned by (and only valid for the
    /// lifetime of) this provider, so it's copied into a fresh, independently-owned DdsQos
    /// before returning - same as DdsQos's own Clone - rather than tying the result's
    /// lifetime to self.
    pub fn get_qos(&self, kind: dds_qos_kind, key: &str) -> Result<DdsQos, DDSError> {
        let key = CString::new(key).map_err(|_| DDSError::BadParameter)?;
        let mut qos: *const dds_qos_t = std::ptr::null();
        let ret =
            unsafe { dds_qos_provider_get_qos(self.0, kind, key.as_ptr(), &mut qos) };
        if ret == 0 {
            DdsQos::copy_from(qos)
        } else {
            Err(DDSError::from(ret))
        }
    }
}

impl Drop for DdsQosProvider {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { dds_delete_qos_provider(self.0) };
        }
    }
}

#[cfg(test)]
mod dds_qos_provider_tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<dds>
 <qos_library name="lib0">
  <qos_profile name="pro0">
   <datawriter_qos></datawriter_qos>
   <datareader_qos name="rd0"></datareader_qos>
  </qos_profile>
 </qos_library>
</dds>"#;

    #[test]
    fn test_create() {
        DdsQosProvider::create(XML).expect("create provider from valid XML");
    }

    #[test]
    fn test_create_invalid_xml_fails() {
        assert!(DdsQosProvider::create("not xml at all").is_err());
    }

    #[test]
    fn test_get_qos_unnamed_entity() {
        let provider = DdsQosProvider::create(XML).expect("create provider");
        provider
            .get_qos(dds_qos_kind::DDS_WRITER_QOS, "lib0::pro0")
            .expect("look up the profile's unnamed datawriter_qos");
    }

    #[test]
    fn test_get_qos_named_entity() {
        let provider = DdsQosProvider::create(XML).expect("create provider");
        provider
            .get_qos(dds_qos_kind::DDS_READER_QOS, "lib0::pro0::rd0")
            .expect("look up the profile's named datareader_qos");
    }

    #[test]
    fn test_get_qos_wrong_kind_fails() {
        let provider = DdsQosProvider::create(XML).expect("create provider");
        // pro0 has no participant_qos entry.
        assert!(provider
            .get_qos(dds_qos_kind::DDS_PARTICIPANT_QOS, "lib0::pro0")
            .is_err());
    }

    #[test]
    fn test_get_qos_unknown_key_fails() {
        let provider = DdsQosProvider::create(XML).expect("create provider");
        assert!(provider
            .get_qos(dds_qos_kind::DDS_WRITER_QOS, "lib0::no_such_profile")
            .is_err());
    }

    #[test]
    fn test_create_scoped() {
        DdsQosProvider::create_scoped(XML, "lib0::pro0")
            .expect("create scoped provider matching an existing profile");
    }
}
