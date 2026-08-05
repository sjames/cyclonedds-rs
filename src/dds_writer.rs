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

use cyclonedds_sys::*;
use std::convert::From;
use std::ffi::c_void;
use std::ptr::NonNull;

pub use cyclonedds_sys::{ DdsEntity};
use std::marker::PhantomData;
use crate::SampleBuffer;

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsWritable, Entity};
use crate::serdes::{mark_loaned, mark_raw_loaned, unmark_loaned, unmark_raw_loaned, Sample, TopicType};

pub struct WriterBuilder<T: TopicType> {
    maybe_qos: Option<DdsQos>,
    maybe_listener: Option<DdsListener>,
    phantom : PhantomData<T>,
}

impl <T>WriterBuilder<T> where T: TopicType + 'static {
    pub fn new() -> Self {
        Self {
            maybe_qos: None,
            maybe_listener: None,
            phantom: PhantomData,
        }
    }

    pub fn with_qos(mut self, qos : DdsQos) -> Self {
        self.maybe_qos = Some(qos);
        self
    }

    pub fn with_listener(mut self, listener : DdsListener) -> Self {
        self.maybe_listener = Some(listener);
        self
    }

    pub fn create(self,  
        entity: &dyn DdsWritable,
        topic: DdsTopic<T>) -> Result<DdsWriter<T>, DDSError> {
            DdsWriter::create(entity, topic, self.maybe_qos, self.maybe_listener)
        }
}

pub enum LoanedInner<T: Sized + TopicType + 'static> {
    Uninitialized(NonNull<T>, DdsEntity),
    Initialized(NonNull<T>, DdsEntity),
    Empty,
}

pub struct Loaned<T: Sized + TopicType + 'static> {
    inner : LoanedInner<T>
}

impl <T> Loaned<T>
where T: Sized + TopicType + 'static {
    pub fn as_mut_ptr(&mut self) -> Option<*mut T> {
        match self.inner {
            LoanedInner::Uninitialized(p, _) => Some(p.as_ptr()),
            LoanedInner::Initialized(p, _) => Some(p.as_ptr()),
            LoanedInner::Empty => None,
        }
    }

    // Must mutate `self` in place rather than building a new Loaned and returning it: the
    // latter drops the original `self` at the end of this function, and Drop unconditionally
    // calls dds_return_loan - which would return the loan to cyclone before the caller ever
    // gets to write() it.
    pub fn assume_init(mut self) -> Self {
        if let LoanedInner::Uninitialized(p, e) = &self.inner {
            let p = *p;
            let e = e.clone();
            self.inner = LoanedInner::Initialized(p, e);
        }
        self
    }
}

impl<T> Drop for Loaned<T>
where T : Sized + TopicType + 'static {
    fn drop(&mut self) {
        let (mut p_sample, entity) = match &mut self.inner {
            LoanedInner::Uninitialized(p, entity) => (p.as_ptr(),Some(entity)),
            LoanedInner::Initialized(p, entity) => (p.as_ptr(),Some(entity)),
            LoanedInner::Empty => (std::ptr::null_mut(), None),
        };

        if let Some(entity) = entity {
            // Must happen after the matching dds_write() (inside return_loan()) has run, since
            // that's when from_sample checks this - which it does. Drop always runs strictly
            // after return_loan()'s body (and hence its dds_write() call) has returned.
            unmark_loaned::<T>(p_sample as usize);
            let voidpp:*mut *mut T= &mut p_sample;
            let voidpp = voidpp as *mut *mut c_void;
            unsafe {dds_return_loan(entity.entity(),voidpp,1)};
        }
    }
}

// A DdsWriter::loan_of_size()/loan_serialized() loan: a raw, uninitialized shared-memory
// buffer for building a pre-serialized (CDR) sample, as opposed to Loaned<T>'s typed `*mut T`
// buffer. Needs its own type rather than reusing Loaned<T> - the buffer isn't a valid T until
// it's been filled with serialized bytes and published, so there's no in-place `*mut T` to
// hand out.
pub enum RawLoanInner<T: Sized + TopicType + 'static> {
    Filled(NonNull<u8>, u32, DdsEntity, PhantomData<T>),
    Empty,
}

pub struct RawLoan<T: Sized + TopicType + 'static> {
    inner: RawLoanInner<T>,
}

impl<T> RawLoan<T>
where
    T: Sized + TopicType + 'static,
{
    /// The raw buffer to fill with serialized bytes before calling
    /// DdsWriter::return_raw_loan(). None once the loan has been published.
    pub fn as_mut_slice(&mut self) -> Option<&mut [u8]> {
        match &mut self.inner {
            RawLoanInner::Filled(p, len, _, _) => {
                Some(unsafe { std::slice::from_raw_parts_mut(p.as_ptr(), *len as usize) })
            }
            RawLoanInner::Empty => None,
        }
    }
}

impl<T> Drop for RawLoan<T>
where
    T: Sized + TopicType + 'static,
{
    fn drop(&mut self) {
        // Same shape as Drop for Loaned<T> above: a loan abandoned without ever being
        // returned via return_raw_loan() must still be handed back to cyclone here.
        if let RawLoanInner::Filled(p, _, entity, _) = &self.inner {
            unmark_raw_loaned::<T>(p.as_ptr() as usize);
            let mut p_sample = p.as_ptr() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut p_sample;
            unsafe { dds_return_loan(entity.entity(), voidpp, 1) };
        }
    }
}

#[derive(Clone)]
pub struct DdsWriter<T: Sized + TopicType>(
    DdsEntity,
    Option<DdsListener>,
    PhantomData<T>,
);

impl<'a, T> DdsWriter<T>
where
    T: Sized + TopicType + 'static,
{
    pub fn create(
        entity: &dyn DdsWritable,
        topic: DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_writer(
                entity.entity().entity(),
                topic.entity().entity(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsWriter(
                    DdsEntity::new(w),
                    maybe_listener,
                    PhantomData,
                ))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn write_to_entity(entity: &DdsEntity, msg: std::sync::Arc<T>) -> Result<(), DDSError> {
        unsafe {
            let sample = Sample::<T>::from(msg);
            let sample = &sample as *const Sample<T>;
            let sample = sample as *const ::std::os::raw::c_void;
            let ret = dds_write(entity.entity(), sample);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn write(&mut self, msg: std::sync::Arc<T>) -> Result<(), DDSError> {
        Self::write_to_entity(&self.0, msg)

    }

    // Write msg and dispose the instance in one call. Goes through the same SDK_DATA/
    // Sample<T> convention as write() (dds_writedispose isn't a key-only operation - it
    // publishes new data and disposes the instance atomically), unlike dispose() and
    // unregister_instance() below.
    pub fn writedispose(&mut self, msg: std::sync::Arc<T>) -> Result<(), DDSError> {
        unsafe {
            let sample = Sample::<T>::from(msg);
            let sample = &sample as *const Sample<T>;
            let sample = sample as *const ::std::os::raw::c_void;
            let ret = dds_writedispose(self.0.entity(), sample);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    // Dispose the instance identified by msg's key fields; other fields are ignored. Only
    // the key is read, synchronously, before this returns (see the SDK_KEY branch of
    // from_sample in serdes.rs) - no allocation or retained reference is needed, unlike
    // write()/writedispose().
    pub fn dispose(&mut self, msg: &T) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_dispose(self.0.entity(), msg as *const T as *const c_void);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    // Unregister this writer's ownership of the instance identified by msg's key fields;
    // other fields are ignored. See dispose() above for why this takes a plain reference.
    pub fn unregister_instance(&mut self, msg: &T) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_unregister_instance(self.0.entity(), msg as *const T as *const c_void);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    // Loan memory buffers for zero copy operation. Only supported for fixed size types
    pub fn loan(&mut self) -> Result<Loaned<T>, DDSError> {

        if !T::is_fixed_size() {
            // Loaning is not supported for types that are not fixed size
            return Err(DDSError::Unsupported)
        }

        let mut p_sample : *mut T = std::ptr::null_mut();
        let voidpp:*mut *mut T= &mut p_sample;
        let voidpp = voidpp as *mut *mut c_void;
        let res = unsafe {
            dds_request_loan(self.0.entity(), voidpp)
        };
        if res == 0 {
            mark_loaned::<T>(p_sample as usize);
            Ok(Loaned { inner: LoanedInner::Uninitialized( NonNull::new(p_sample).unwrap(),  self.entity().clone()) })
        } else {
            Err(DDSError::from(res))
        } 
    }

     // Return the loaned buffer.  If the buffer was initialized, then write the data to be published
     pub fn return_loan(&mut self, mut buffer: Loaned<T>) -> Result<(),DDSError> {
        let res = match &mut buffer.inner {

            LoanedInner::Uninitialized(p,entity) => {
                let mut p_sample = p.as_ptr();
                let voidpp:*mut *mut T= &mut p_sample;
                let voidpp = voidpp as *mut *mut c_void;
                unsafe {dds_return_loan(entity.entity(),voidpp,1)}
            },
            LoanedInner::Initialized(p, entity) => {
                let p_sample = p.as_ptr();
                unsafe {dds_write(entity.entity(), p_sample as * const c_void)}
            }
            LoanedInner::Empty => 0,
        };

        // dds_write() "takes over the loan" (dds_public_loan_api.h) - dds_return_loan must
        // NOT be called again on this pointer afterward, or the chunk goes back to the
        // shared-memory pool while cyclone may still be using it (e.g. RELIABLE retransmit
        // history), letting a concurrent loan() on another writer recycle and overwrite it
        // before it's actually delivered. Consume the Initialized case here so Drop - which
        // unconditionally calls dds_return_loan - sees Empty and does nothing.
        if res == 0 {
            if let LoanedInner::Initialized(p, _) = &buffer.inner {
                unmark_loaned::<T>(p.as_ptr() as usize);
                buffer.inner = LoanedInner::Empty;
            }
            Ok(())
        } else {
            Err(DDSError::from(res))
        }

    }

    // Request a raw, uninitialized shared-memory loan of exactly `size` bytes, for filling
    // with a pre-serialized (CDR) sample and publishing via return_raw_loan(). Unlike loan(),
    // this doesn't require T::is_fixed_size() - the buffer isn't a `*mut T` at all, it's
    // meant to hold already-serialized bytes. Still requires shared memory to be available
    // for this writer, same as dds_request_loan - dds_request_loan_of_size
    // (dds_public_loan_api.h) has no heap-loan fallback.
    pub fn loan_of_size(&mut self, size: u32) -> Result<RawLoan<T>, DDSError> {
        if T::is_fixed_size() {
            // Mirrors loan()'s opposite restriction (fixed-size only). A matching reader's
            // from_psmx has no cross-process way to tell a raw-loan buffer of pre-serialized
            // bytes apart from a regular loan's raw T struct other than by T::is_fixed_size()
            // itself (see from_psmx in serdes.rs) - mixing both loan kinds for the same T
            // would break that. Fixed-size types should use loan() instead, which is cheaper
            // anyway (no serialization).
            return Err(DDSError::Unsupported);
        }

        let mut p_sample: *mut c_void = std::ptr::null_mut();
        let res = unsafe { dds_request_loan_of_size(self.0.entity(), size as usize, &mut p_sample) };
        if res == 0 {
            mark_raw_loaned::<T>(p_sample as usize, size);
            Ok(RawLoan {
                inner: RawLoanInner::Filled(
                    NonNull::new(p_sample as *mut u8).unwrap(),
                    size,
                    self.entity().clone(),
                    PhantomData,
                ),
            })
        } else {
            Err(DDSError::from(res))
        }
    }

    // Publish a raw loan filled via loan_of_size()'s as_mut_slice(). Same double-return
    // hazard and fix as return_loan() above: dds_write() takes over the loan on success, so
    // the loan is consumed here rather than left for Drop to hand back a second time.
    pub fn return_raw_loan(&mut self, mut loan: RawLoan<T>) -> Result<(), DDSError> {
        let res = match &loan.inner {
            RawLoanInner::Filled(p, _, entity, _) => unsafe {
                dds_write(entity.entity(), p.as_ptr() as *const c_void)
            },
            RawLoanInner::Empty => 0,
        };

        if res == 0 {
            if let RawLoanInner::Filled(p, _, _, _) = &loan.inner {
                unmark_raw_loaned::<T>(p.as_ptr() as usize);
                loan.inner = RawLoanInner::Empty;
            }
            Ok(())
        } else {
            Err(DDSError::from(res))
        }
    }

    // Serialize msg directly into a shared-memory loan and publish it. When SHM is active
    // and T isn't memcpy-safe (has heap-owned fields, so loan()/return_loan()'s raw-struct
    // path isn't available), a plain write() still ends up with cyclone doing get_size() +
    // allocate a PSMX loan + serialize into it internally - this does the same thing but
    // serializes straight into the final shared-memory buffer instead of into an
    // intermediate heap Vec that write()'s path would otherwise produce and then have
    // memcpy'd into that same PSMX loan. Key hashing/matching behave identically to write():
    // see raw_loaned_size's use in from_loaned_sample and from_sample in serdes.rs.
    pub fn loan_serialized(&mut self, msg: &T) -> Result<(), DDSError> {
        let size = crate::serdes::padded_cdr_size(msg);
        let mut loan = self.loan_of_size(size)?;
        let buf = loan
            .as_mut_slice()
            .expect("loan_of_size() always returns a Filled loan on success");

        let bytes =
            crate::serdes::serialize_type(msg, Some(size)).map_err(|_| DDSError::DdsError)?;
        if bytes.len() > buf.len() {
            // Shouldn't happen - padded_cdr_size() and serialize_type()'s own rounding use
            // the same padding - but never write out of the loan's bounds if it does.
            return Err(DDSError::DdsError);
        }
        buf[..bytes.len()].copy_from_slice(&bytes);

        self.return_raw_loan(loan)
    }

    pub fn set_listener(&mut self, listener: DdsListener) -> Result<(), DDSError> {
        unsafe {
            let refl = &listener;
            let rc = dds_set_listener(self.0.entity(), refl.into());
            if rc == 0 {
                self.1 = Some(listener);
                Ok(())
            } else {
                Err(DDSError::from(rc))
            }
        }
    }
}

impl<'a, T> Entity for DdsWriter<T>
where
    T: std::marker::Sized + TopicType,
{
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

impl<'a, T> Drop for DdsWriter<T>
where
    T: std::marker::Sized + TopicType,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0.entity()).into();
            if DDSError::DdsOk != ret && DDSError::AlreadyDeleted != ret {
                //panic!("cannot delete Writer: {}", ret);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use core::panic;
    use std::{time::Duration, sync::Arc, ops::Deref};

    use crate::{DdsParticipant, DdsSubscriber, DdsReader};
    use super::*;
    use crate::{DdsPublisher, DdsWriter};
    
    use cdds_derive::{Topic, TopicFixedSize};
    use serde_derive::{Deserialize, Serialize};
    use tokio::runtime::Runtime;

    const cyclone_shm_config : &str = r###"<?xml version="1.0" encoding="UTF-8" ?>
    <CycloneDDS xmlns="https://cdds.io/config"
                xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                xsi:schemaLocation="https://cdds.io/config https://raw.githubusercontent.com/eclipse-cyclonedds/cyclonedds/iceoryx/etc/cyclonedds.xsd">
        <Domain id="any">
            <General>
                <Interfaces>
                    <PubSubMessageExchange type="iox" config="LOG_LEVEL=INFO;"/>
                </Interfaces>
            </General>
        </Domain>
    </CycloneDDS>"###;


    #[repr(C)]
    #[derive(Serialize,Deserialize,Debug, PartialEq, Clone)]
    enum Position {
        Front,
        Back,
    }

    impl Default for Position {
        fn default() -> Self {
            Self::Front
        }
    }
    
    #[derive(Serialize,Deserialize,TopicFixedSize, Debug, PartialEq)]
    struct TestTopic {
        a : u32,
        b : u16,
        c: [u8;10],
        d : [u8;15],
        #[topic_key]
        e : u32,
        #[topic_key_enum]
        pos : Position,
    }

    impl Default for TestTopic {
        fn default() -> Self {
            Self {
                a : 10,
                b : 20,
                c : [0,0,0,0,0,0,0,0,0,0],
                d : [1,2,3,4,5,6,7,8,9,0,1,2,3,4,5],
                e : 0,
                pos : Position::default(),
            }
        }
    }

    #[derive(Serialize,Deserialize,Topic, Debug, PartialEq)]
    struct AnotherTopic {
        pub value : u32,
        pub name : String,
        pub arr : [String;2],
        pub vec : Vec<String>,
        #[topic_key]
        pub key : u32,
    }

    impl Default for AnotherTopic {
        fn default() -> Self {
            assert!(Self::has_key());
            Self {
                value : 42,
                name : "the answer".to_owned(),
                arr : ["one".to_owned(), "two".to_owned()],
                vec : vec!["Hello".to_owned(), "world".to_owned()],
                key : 0,
            }
    }
    }

    // Requires a running iox-roudi (Iceoryx's shared-memory broker). Not something `cargo
    // test` can bring up on its own, so this stays opt-in: `cargo test -- --ignored test_loan`.
    #[test]
    #[ignore = "requires iox-roudi running"]
    fn test_loan() {
        // Make sure iox-roudi is running
        std::env::set_var("CYCLONEDDS_URI", cyclone_shm_config);

        // CycloneDDS caches a domain's config the first time it's used in a process, and
        // DdsParticipant::create(None, ..) requests DDS_DOMAIN_DEFAULT, which joins the
        // lowest-numbered domain that already exists (falling back to domain 0). Every other
        // test in this crate creates its participant the same way, so sharing that default
        // domain would mean this test's CYCLONEDDS_URI/SharedMemory config might never
        // actually apply if another test happened to stand domain 0 up first in the same
        // process (e.g. `cargo test -- --include-ignored`). A dedicated domain id guarantees
        // this test always creates a fresh domain, so its config always takes effect - the
        // XML's <Domain id="any"> already matches any numeric domain id.
        const TEST_LOAN_DOMAIN_ID: u32 = 42;
        let participant = DdsParticipant::create(Some(TEST_LOAN_DOMAIN_ID), None, None).unwrap();

        let topic = TestTopic::create_topic(&participant, Some("test_topic"), None, None).unwrap();
        let another_topic = AnotherTopic::create_topic(&participant, None, None, None).unwrap();

        let publisher = DdsPublisher::create(&participant, None, None).unwrap();

        let mut writer = DdsWriter::create(&publisher, topic.clone(), None, None).unwrap();
        let mut another_writer = DdsWriter::create(&publisher, another_topic.clone(), None, None).unwrap();

        // this writer does not have a fixed size. Loan should fail
        
        if let Ok(r) = another_writer.loan() {
            panic!("This must fail");
        }

        let subscriber = DdsSubscriber::create(&participant, None, None).unwrap();
        let reader = DdsReader::create_async(&subscriber, topic, None).unwrap();
        let another_reader = DdsReader::create_async(&subscriber, another_topic, None).unwrap();

        let rt = Runtime::new().unwrap();

        let _result = rt.block_on(async {
            
          

            let _another_task = tokio::spawn(async move {
                let mut samples = TestTopic::create_sample_buffer(5);
                if let Ok(t) = reader.take(&mut samples).await {
                    assert_eq!(t,1);
                    for s in samples.iter() {

                        println!("Got sample {:?}", s);
                    }
                   
                } else {
                    panic!("reader get failed");
                }
            });

            // add a delay to make sure the data is not ready immediately
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

             let mut loaned = writer.loan().unwrap(); 

             let ptr = loaned.as_mut_ptr().unwrap();
             let topic = TestTopic::default();
            
             unsafe {ptr.write(topic)};
             let loaned = loaned.assume_init();
             writer.return_loan(loaned).unwrap();

            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        });

    }

    

}
