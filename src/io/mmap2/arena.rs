use std::{io, mem, ptr, slice, sync::Arc};

use crate::buffer;
use crate::device::Handle;
use crate::io::arena::Arena as ArenaTrait;
use crate::memory::Memory;
use crate::v4l2;
use crate::v4l_sys::*;
use crate::buffer::Buffer;

use std::mem::ManuallyDrop;

/// Manage mapped buffers
///
/// All buffers are unmapped in the Drop impl.
/// In case of errors during unmapping, we panic because there is memory corruption going on.
pub struct Arena {
    handle: Arc<Handle>,
    bufs: Vec<Arc<ManuallyDrop<Vec<u8>>>>,
    buf_sizes: Vec<usize>,
    buf_type: buffer::Type,
}

impl Arena {
    /// Returns a new buffer manager instance
    ///
    /// You usually do not need to use this directly.
    /// A MappedBufferStream creates its own manager instance by default.
    ///
    /// # Arguments
    ///
    /// * `handle` - Device handle to get its file descriptor
    /// * `buf_type` - Type of the buffers
    pub fn new(handle: Arc<Handle>, buf_type: buffer::Type) -> Self {
        Arena {
            handle,
            bufs: Vec::new(),
            buf_sizes: Vec::new(),
            buf_type,
        }
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        if self.bufs.is_empty() {
            // nothing to do
            return;
        }

        if let Err(e) = self.release() {
            if let Some(code) = e.raw_os_error() {
                // ENODEV means the file descriptor wrapped in the handle became invalid, most
                // likely because the device was unplugged or the connection (USB, PCI, ..)
                // broke down. Handle this case gracefully by ignoring it.
                if code == 19 {
                    /* ignore */
                    return;
                }
            }

            panic!("{:?}", e)
        }
    }
}

impl ArenaTrait for Arena {
    type Buffer = Arc<ManuallyDrop<Vec<u8>>>;

    fn allocate(&mut self, count: u32) -> io::Result<u32> {
        let mut v4l2_reqbufs: v4l2_requestbuffers;
        unsafe {
            v4l2_reqbufs = mem::zeroed();
            v4l2_reqbufs.type_ = self.buf_type as u32;
            v4l2_reqbufs.count = count;
            v4l2_reqbufs.memory = Memory::Mmap as u32;
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_REQBUFS,
                &mut v4l2_reqbufs as *mut _ as *mut std::os::raw::c_void,
            )?;
        }

        for i in 0..v4l2_reqbufs.count {
            let mut v4l2_buf: v4l2_buffer;
            unsafe {
                v4l2_buf = mem::zeroed();
                v4l2_buf.type_ = self.buf_type as u32;
                v4l2_buf.memory = Memory::Mmap as u32;
                v4l2_buf.index = i;
                v4l2::ioctl(
                    self.handle.fd(),
                    v4l2::vidioc::VIDIOC_QUERYBUF,
                    &mut v4l2_buf as *mut _ as *mut std::os::raw::c_void,
                )?;

                let ptr = v4l2::mmap(
                    ptr::null_mut(),
                    v4l2_buf.length as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    self.handle.fd(),
                    v4l2_buf.m.offset as libc::off_t,
                )?;

                // FIXME: Vec will try to clear this data, but it shouldn't. How do I tell it that ?
                let data = unsafe { Vec::from_raw_parts(ptr as *mut u8, v4l2_buf.length as usize, v4l2_buf.length as usize) };
                let data = mem::ManuallyDrop::new(data);
                self.bufs.push(Arc::new(data));
            }
        }

        Ok(v4l2_reqbufs.count)
    }

    fn release(&mut self) -> io::Result<()> {
        for buf in &self.bufs {
            unsafe {
                let data = buf.as_ptr();
                v4l2::munmap(data as *mut core::ffi::c_void, buf.len())?;
            }
        }

        // free all buffers by requesting 0
        let mut v4l2_reqbufs: v4l2_requestbuffers;
        unsafe {
            v4l2_reqbufs = mem::zeroed();
            v4l2_reqbufs.type_ = self.buf_type as u32;
            v4l2_reqbufs.count = 0;
            v4l2_reqbufs.memory = Memory::Mmap as u32;
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_REQBUFS,
                &mut v4l2_reqbufs as *mut _ as *mut std::os::raw::c_void,
            )?;
        }

        self.bufs.clear();
        Ok(())
    }

    fn get(&self, index: usize) -> Option<Self::Buffer> {
        Some(Arc::clone(self.bufs.get(index).unwrap()))
    }

    /*fn get_mut(&mut self, index: usize) -> Option<&mut Self::Buffer> {
        Some(self.bufs.get_mut(index)?)
    }*/

    /*unsafe fn get_unchecked(&self, index: usize) -> &Self::Buffer {
        self.bufs.get_unchecked(index)
    }

    unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut Self::Buffer {
        self.bufs.get_unchecked_mut(index)
    }*/

    fn len(&self) -> usize {
        self.bufs.len()
    }
}
