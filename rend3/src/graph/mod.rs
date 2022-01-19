//! Rendergraph implementation that rend3 uses for all render work scheduling.
//!
//! Start with [RenderGraph::new] and add nodes and then
//! [RenderGraph::execute] to run everything.
//!
//! # High Level Overview
//!
//! The design consists of a series of nodes which have inputs and outputs.
//! These inputs can be render targets, shadow targets, or custom user data. The
//! graph is laid out in order using the inputs/outputs then pruned.
//!
//! Each node is a pile of arbitrary code that can use various resources within
//! the renderer to do work.
//!
//! All work that doesn't interact with the surface is submitted, then the
//! surface is acquired, then all the following work is submitted.
//!
//! # Nodes
//!
//! Nodes are made with [RenderGraphNodeBuilder]. The builder is used to declare
//! all the dependencies of the node ("outside" the node), then
//! [RenderGraphNodeBuilder::build] is called. This takes a callback that
//! contains all the code that will run as part of the node (the "inside").
//!
//! The arguments given to this callback give you all the data you need to do
//! your work, including turning handles-to-dependencies into actual concrete
//! resources.
//!
//! # Renderpasses/Encoders
//!
//! The graph will automatically deduplicate renderpasses, such that if there
//! are two nodes in a row that have a compatible renderpass, they will use the
//! same renderpass. An encoder will not be available if a renderpass is in use.
//! This is intentional as there should be as few renderpasses as possible, so
//! you should separate the code that needs a raw encoder from the code that is
//! using a renderpass.
//!
//! Because renderpasses carry with them a lifetime that can cause problems, two
//! facilities are available.
//!
//! First is the [PassthroughDataContainer] which
//! allows you to take lifetimes of length `'node` and turn them into lifetimes
//! of length `'rpass`. This is commonly used to bring in any state from the
//! outside.
//!
//! Second is the [RpassTemporaryPool]. If, inside the node, you need to create
//! a temporary, you can put that temporary on the pool, and it will
//! automatically have lifetime `'rpass`. The temporary is destroyed right after
//! the renderpass is.

use glam::UVec2;
use rend3_types::{BufferUsages, SampleCount, TextureFormat, TextureUsages};
use wgpu::{Color, TextureView};

use crate::util::typedefs::SsoString;

mod encpass;
#[allow(clippy::module_inception)] // lmao
mod graph;
mod node;
mod passthrough;
mod store;
mod temp;

pub use encpass::*;
pub use graph::*;
pub use node::*;
pub use passthrough::*;
pub use store::*;
pub use temp::*;

#[derive(Debug, Clone)]
pub struct RenderTargetDescriptor {
    pub label: Option<SsoString>,
    pub resolution: UVec2,
    pub samples: SampleCount,
    pub format: TextureFormat,
    pub usage: TextureUsages,
}
impl RenderTargetDescriptor {
    fn to_core(&self) -> RenderTargetCore {
        RenderTargetCore {
            resolution: self.resolution,
            samples: self.samples,
            format: self.format,
            usage: self.usage,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RenderTargetCore {
    pub resolution: UVec2,
    pub samples: SampleCount,
    pub format: TextureFormat,
    pub usage: TextureUsages,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BufferTargetDescriptor {
    pub label: Option<SsoString>,
    pub length: u64,
    pub usage: BufferUsages,
    pub mapped: bool,
}

pub struct ShadowTarget<'a> {
    pub view: &'a TextureView,
    pub offset: UVec2,
    pub size: usize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
enum GraphResource {
    OutputTexture,
    External,
    Texture(usize),
    Shadow(usize),
    Data(usize),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct RenderTargetHandle {
    resource: GraphResource,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ShadowTargetHandle {
    idx: usize,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ShadowArrayHandle;

#[derive(Debug, PartialEq)]
pub struct RenderPassTargets {
    pub targets: Vec<RenderPassTarget>,
    pub depth_stencil: Option<RenderPassDepthTarget>,
}

impl RenderPassTargets {
    pub fn compatible(this: Option<&Self>, other: Option<&Self>) -> bool {
        match (this, other) {
            (Some(this), Some(other)) => {
                let targets_compatible = this.targets.len() == other.targets.len()
                    && this
                        .targets
                        .iter()
                        .zip(other.targets.iter())
                        .all(|(me, you)| me.color == you.color && me.resolve == you.resolve);

                let depth_compatible = match (&this.depth_stencil, &other.depth_stencil) {
                    (Some(this_depth), Some(other_depth)) => this_depth == other_depth,
                    (None, None) => true,
                    _ => false,
                };

                targets_compatible && depth_compatible
            }
            (None, None) => true,
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct RenderPassTarget {
    pub color: DeclaredDependency<RenderTargetHandle>,
    pub clear: Color,
    pub resolve: Option<DeclaredDependency<RenderTargetHandle>>,
}

#[derive(Debug, PartialEq)]
pub struct RenderPassDepthTarget {
    pub target: DepthHandle,
    pub depth_clear: Option<f32>,
    pub stencil_clear: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub enum DepthHandle {
    RenderTarget(DeclaredDependency<RenderTargetHandle>),
    Shadow(ShadowTargetHandle),
}
