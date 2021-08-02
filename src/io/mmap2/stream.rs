use std::{io, mem, sync::Arc, rc::Rc};

use crate::buffer::{Metadata, Type, Buffer};
use crate::device::{Device, Handle};
use crate::io::arena::Arena as ArenaTrait;
use crate::io::mmap2::arena::Arena;
use crate::memory::Memory;
use crate::v4l2;
use crate::v4l_sys::*;
use std::mem::ManuallyDrop;

pub struct Stream {
    stream_int: Arc<StreamInt>
}

impl Stream {
    pub fn new(dev: &Device, buf_type: Type) -> io::Result<Self> {
        Stream::with_buffers(dev, buf_type, 4)
    }

    pub fn with_buffers(dev: &Device, buf_type: Type, buf_count: u32) -> io::Result<Self> {
        let stream = StreamInt::with_buffers(dev, buf_type, buf_count);

        let stream = match stream {
            Ok(s) => s,
            Err(e) => return Err(e),
        };

        Ok(Stream {
            stream_int: Arc::new(stream)
        })
    }

    pub fn start(&self) -> io::Result<()> {
        self.stream_int.start()
    }

    pub fn stop(&self) -> io::Result<()> {
        self.stream_int.start()
    }

    pub fn next(&self) -> io::Result<Buffer> {
        Buffer::from_queue(Arc::clone(&self.stream_int))
    }
}

/// Stream of mapped buffers
///
/// An arena instance is used internally for buffer handling.
pub struct StreamInt {
    handle: Arc<Handle>,
    arena: Arena,
    arena_index: usize,
    buf_type: Type,
    buf_meta: Vec<Metadata>,

    active: bool,
}

impl StreamInt {
    /// Returns a stream for frame capturing
    ///
    /// # Arguments
    ///
    /// * `dev` - Capture device ref to get its file descriptor
    /// * `buf_type` - Type of the buffers
    ///
    /// # Example
    ///
    /// ```
    /// use v4l::buffer::Type;
    /// use v4l::device::Device;
    /// use v4l::io::mmap::Stream;
    ///
    /// let dev = Device::new(0);
    /// if let Ok(dev) = dev {
    ///     let stream = Stream::new(&dev, Type::VideoCapture);
    /// }
    /// ```
    pub fn new(dev: &Device, buf_type: Type) -> io::Result<Self> {
        StreamInt::with_buffers(dev, buf_type, 4)
    }

    pub fn with_buffers(dev: &Device, buf_type: Type, buf_count: u32) -> io::Result<Self> {
        let mut arena = Arena::new(dev.handle(), buf_type);
        let count = arena.allocate(buf_count)?;
        let mut buf_meta = Vec::new();
        buf_meta.resize(count as usize, Metadata::default());

        Ok(StreamInt {
            handle: dev.handle(),
            arena,
            arena_index: 0,
            buf_type,
            buf_meta,
            active: false,
        })
    }

    fn start(&self) -> io::Result<()> {
        unsafe {
            let mut typ = self.buf_type as u32;
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_STREAMON,
                &mut typ as *mut _ as *mut std::os::raw::c_void,
            )?;
        }

        for index in 0..self.arena.len() {
            self.queue(index)?;
        }

        //self.active = true;
        Ok(())
    }

    fn stop(&self) -> io::Result<()> {
        unsafe {
            let mut typ = self.buf_type as u32;
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_STREAMOFF,
                &mut typ as *mut _ as *mut std::os::raw::c_void,
            )?;
        }

        //self.active = false;
        Ok(())
    }

    pub fn queue(&self, index: usize) -> io::Result<()> {
        let mut v4l2_buf: v4l2_buffer;
        unsafe {
            v4l2_buf = mem::zeroed();
            v4l2_buf.type_ = self.buf_type as u32;
            v4l2_buf.memory = Memory::Mmap as u32;
            v4l2_buf.index = index as u32;
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_QBUF,
                &mut v4l2_buf as *mut _ as *mut std::os::raw::c_void,
            )?;
        }

        Ok(())
    }

    pub fn dequeue(&self) -> io::Result<(usize, Metadata)> {
        let mut v4l2_buf: v4l2_buffer;
        unsafe {
            v4l2_buf = mem::zeroed();
            v4l2_buf.type_ = self.buf_type as u32;
            v4l2_buf.memory = Memory::Mmap as u32;
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_DQBUF,
                &mut v4l2_buf as *mut _ as *mut std::os::raw::c_void,
            )?;
        }
        let arena_index = v4l2_buf.index as usize;

        let meta = Metadata {
            bytesused: v4l2_buf.bytesused,
            flags: v4l2_buf.flags.into(),
            field: v4l2_buf.field,
            timestamp: v4l2_buf.timestamp.into(),
            sequence: v4l2_buf.sequence,
        };

        Ok((arena_index, meta))
    }

    pub fn get(&self, index: usize) -> Option<Arc<ManuallyDrop<Vec<u8>>>> {
        let arc = self.arena.get(index)?;
        //Some(Arc::clone(&arc))
        Some(arc)
    }

    fn get_meta(&self, index: usize) -> Option<&Metadata> {
        self.buf_meta.get(index)
    }
}

impl Drop for StreamInt {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
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
