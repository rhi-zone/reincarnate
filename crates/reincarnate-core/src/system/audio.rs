/// Opaque handle to a decoded audio buffer.
pub type BufferId = u32;

/// Opaque handle to a DSP node in the audio graph (0 = master output).
pub type NodeId = u32;

/// Opaque handle to a playing voice instance (0 = invalid).
pub type VoiceId = u32;

/// DSP node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Gain,
    Pan,
    LowPass,
    HighPass,
    BandPass,
    Notch,
    Compressor,
    Reverb,
    Delay,
    Mixer,
}

/// Named parameter for a DSP node. All values are f32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamKind {
    Gain,
    Pan,
    Cutoff,
    Resonance,
    WetMix,
    Decay,
    DelayTime,
    Feedback,
    Threshold,
    Ratio,
    Attack,
    Release,
    Knee,
}

/// Arguments for [`Audio::play`]. A struct avoids exceeding the 7-arg limit
/// while remaining stack-allocated (no heap cost in Rust, unlike TS `{}`).
#[derive(Debug, Clone, Copy)]
pub struct PlayParams {
    pub buffer: BufferId,
    pub sink: NodeId,
    pub loop_: bool,
    pub gain: f32,
    pub pitch: f32,
    pub pan: f32,
    pub offset: f32,
}

/// Audio platform trait — node graph-based audio system.
///
/// Signal graph: Buffer → Voice(gain+pan) → NodeGraph → Master
///
/// Node 0 is the implicit master output, always valid after initialization.
/// Voices route through a user-defined DAG of DSP nodes.
///
/// Two tiers:
///   Setup tier  — graph construction (create_node, connect, set_node_param)
///   Hot tier    — voice lifecycle (play, stop, pause, set_voice_gain, ...)
///
/// All hot-tier methods take only primitive arguments (no allocation at call site).
pub trait Audio {
    // ---- Setup tier ----

    /// Decode an audio file from a URL and return its BufferId.
    fn load_buffer(&mut self, url: &str) -> BufferId;

    /// Create a DSP node of the given kind. Returns its NodeId.
    fn create_node(&mut self, kind: NodeKind) -> NodeId;

    /// Add a directed edge in the node graph: from.output → to.input.
    fn connect(&mut self, from: NodeId, to: NodeId);

    /// Remove a directed edge. Safe to call if not connected.
    fn disconnect(&mut self, from: NodeId, to: NodeId);

    /// Set or animate a node parameter. Panics/errors if kind is invalid for this node.
    fn set_node_param(&mut self, node: NodeId, kind: ParamKind, value: f32, fade_ms: f32);

    /// Read the current value of a node parameter.
    fn get_node_param(&self, node: NodeId, kind: ParamKind) -> f32;

    // ---- Hot tier: voice lifecycle ----

    /// Play a buffer, routing output through sink. Returns a VoiceId.
    fn play(&mut self, params: PlayParams) -> VoiceId;

    fn stop(&mut self, voice: VoiceId);
    fn stop_all(&mut self);
    fn pause(&mut self, voice: VoiceId);
    fn resume(&mut self, voice: VoiceId);
    fn resume_all(&mut self);
    fn is_playing(&self, voice: VoiceId) -> bool;
    fn is_paused(&self, voice: VoiceId) -> bool;

    // ---- Hot tier: per-voice parameter control ----

    fn set_voice_gain(&mut self, voice: VoiceId, gain: f32, fade_ms: f32);
    fn get_voice_gain(&self, voice: VoiceId) -> f32;
    fn set_voice_pitch(&mut self, voice: VoiceId, pitch: f32, fade_ms: f32);
    fn get_voice_pitch(&self, voice: VoiceId) -> f32;
    fn set_voice_pan(&mut self, voice: VoiceId, pan: f32);
    fn get_voice_pan(&self, voice: VoiceId) -> f32;

    /// Convenience: set master gain (equivalent to set_node_param(0, Gain, gain, 0.0)).
    fn set_master_gain(&mut self, gain: f32);

    fn get_position(&self, voice: VoiceId) -> f32;
    fn set_position(&mut self, voice: VoiceId, pos: f32);
    fn sound_length(&self, buffer: BufferId) -> f32;

    // ---- Node-level bulk operations ----

    /// Stop all voices currently routed directly to node.
    fn stop_node(&mut self, node: NodeId);
    /// Pause all non-paused voices currently routed directly to node.
    fn pause_node(&mut self, node: NodeId);
    /// Resume all paused voices currently routed directly to node.
    fn resume_node(&mut self, node: NodeId);
}
