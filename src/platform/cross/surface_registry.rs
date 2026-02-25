use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// An opaque identifier for a registered WGPU surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub(crate) u64);

#[allow(dead_code)]
struct DoubleBuffer {
    textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    front: usize,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// Thread-safe registry of all active WGPU surfaces.
/// Maps `SurfaceId` to double-buffered texture pairs.
pub struct SurfaceRegistry {
    surfaces: Mutex<HashMap<SurfaceId, DoubleBuffer>>,
    next_id: AtomicU64,
}

impl SurfaceRegistry {
    pub fn new() -> Self {
        Self {
            surfaces: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a new double-buffered surface. Returns its `SurfaceId`.
    pub fn create(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> SurfaceId {
        let id = SurfaceId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let db = Self::create_double_buffer(device, width, height, format);
        self.surfaces.lock().unwrap().insert(id, db);
        id
    }

    /// Swap front and back buffers (pointer swap, no GPU work).
    pub fn swap_buffers(&self, id: SurfaceId) {
        if let Some(db) = self.surfaces.lock().unwrap().get_mut(&id) {
            db.front = 1 - db.front;
        }
    }

    /// Resize both buffers, creating new textures.
    pub fn resize(
        &self,
        device: &wgpu::Device,
        id: SurfaceId,
        width: u32,
        height: u32,
    ) {
        let mut surfaces = self.surfaces.lock().unwrap();
        if let Some(db) = surfaces.get_mut(&id) {
            if db.width == width && db.height == height {
                return;
            }
            let new_db = Self::create_double_buffer(device, width, height, db.format);
            *db = new_db;
        }
    }

    /// Get the front buffer's `TextureView` (what the renderer reads from).
    pub fn front_view(&self, id: SurfaceId) -> Option<wgpu::TextureView> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|db| {
            db.textures[db.front].create_view(&wgpu::TextureViewDescriptor::default())
        })
    }

    /// Get the back buffer's `Texture` (what external code renders into).
    #[allow(dead_code)]
    pub fn back_texture(&self, _id: SurfaceId) -> Option<wgpu::Texture> {
        // wgpu::Texture is internally Arc'd, so we can't just hand it out.
        // Instead we'll provide a view via back_view().
        None
    }

    /// Get the back buffer's `TextureView` for use as a render target.
    pub fn back_view(&self, id: SurfaceId) -> Option<wgpu::TextureView> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|db| {
            let back = 1 - db.front;
            db.textures[back].create_view(&wgpu::TextureViewDescriptor::default())
        })
    }

    /// Get the current size of a surface.
    #[allow(dead_code)]
    pub fn size(&self, id: SurfaceId) -> Option<(u32, u32)> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|db| (db.width, db.height))
    }

    /// Get the texture format for a surface.
    #[allow(dead_code)]
    pub fn format(&self, id: SurfaceId) -> Option<wgpu::TextureFormat> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|db| db.format)
    }

    /// Remove a surface from the registry.
    pub fn remove(&self, id: SurfaceId) {
        self.surfaces.lock().unwrap().remove(&id);
    }

    fn create_double_buffer(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> DoubleBuffer {
        let w = width.max(1);
        let h = height.max(1);

        let create_texture = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };

        let tex0 = create_texture("surface_buffer_0");
        let tex1 = create_texture("surface_buffer_1");
        let view0 = tex0.create_view(&wgpu::TextureViewDescriptor::default());
        let view1 = tex1.create_view(&wgpu::TextureViewDescriptor::default());

        DoubleBuffer {
            textures: [tex0, tex1],
            views: [view0, view1],
            front: 0,
            width: w,
            height: h,
            format,
        }
    }
}
