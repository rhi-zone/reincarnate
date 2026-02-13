use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// Number of event type categories in GameMaker.
pub const EVENT_TYPE_COUNT: usize = 12;

/// Event type indices.
pub mod event_type {
    pub const CREATE: usize = 0;
    pub const DESTROY: usize = 1;
    pub const ALARM: usize = 2;
    pub const STEP: usize = 3;
    pub const COLLISION: usize = 4;
    pub const KEYBOARD: usize = 5;
    pub const MOUSE: usize = 6;
    pub const OTHER: usize = 7;
    pub const DRAW: usize = 8;
    pub const KEY_PRESS: usize = 9;
    pub const KEY_RELEASE: usize = 10;
    pub const TRIGGER: usize = 11;
}

/// An action within an event sub-entry.
#[derive(Debug)]
pub struct Action {
    /// Library ID (usually 1).
    pub lib_id: u32,
    /// Action ID within the library.
    pub action_id: u32,
    /// Action kind (7 = code execution).
    pub action_kind: u32,
    /// Whether the action uses relative values.
    pub has_relative: bool,
    /// Whether this action is a question (condition).
    pub is_question: bool,
    /// What this action applies to (1 = self, -1 = other, etc.).
    pub applies_to: i32,
    /// Execution type (2 = code).
    pub exec_type: u32,
    /// Reference to the function name string (often empty).
    pub func_name: StringRef,
    /// Index into the CODE chunk's entry list.
    pub code_id: u32,
    /// Number of arguments.
    pub arg_count: u32,
    /// Who (-1 = self).
    pub who: i32,
    /// Whether values are relative.
    pub relative: bool,
    /// Whether the condition is negated.
    pub is_not: bool,
}

/// An event sub-entry (e.g., Create_0, Mouse_11).
#[derive(Debug)]
pub struct EventEntry {
    /// Event subtype (e.g., 0 for Create_0, 11 for Mouse_11).
    pub subtype: u32,
    /// Actions in this event.
    pub actions: Vec<Action>,
}

/// Physics shape type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PhysicsShape {
    Circle = 0,
    Box = 1,
    Custom = 2,
}

impl PhysicsShape {
    fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Circle,
            1 => Self::Box,
            _ => Self::Custom,
        }
    }
}

/// Physics vertex (x, y).
#[derive(Debug, Clone, Copy)]
pub struct PhysicsVertex {
    pub x: f32,
    pub y: f32,
}

/// An object definition in the OBJT chunk.
#[derive(Debug)]
pub struct ObjectEntry {
    /// Reference to the object name string.
    pub name: StringRef,
    /// Sprite index (-1 = none).
    pub sprite_index: i32,
    /// Whether the object is visible.
    pub visible: bool,
    /// Whether the object is solid.
    pub solid: bool,
    /// Depth layer.
    pub depth: i32,
    /// Whether the object persists across rooms.
    pub persistent: bool,
    /// Parent object index (-100 = none).
    pub parent_index: i32,
    /// Mask sprite index (-1 = use own sprite).
    pub mask_index: i32,
    /// Whether physics is enabled.
    pub physics_enabled: bool,
    /// Whether the physics body is a sensor.
    pub physics_sensor: bool,
    /// Physics collision shape type.
    pub physics_shape: PhysicsShape,
    /// Physics density.
    pub physics_density: f32,
    /// Physics restitution (bounciness).
    pub physics_restitution: f32,
    /// Physics collision group.
    pub physics_group: u32,
    /// Physics linear damping.
    pub physics_linear_damping: f32,
    /// Physics angular damping.
    pub physics_angular_damping: f32,
    /// Physics shape vertices.
    pub physics_vertices: Vec<PhysicsVertex>,
    /// Physics friction.
    pub physics_friction: f32,
    /// Physics awake on creation.
    pub physics_awake: bool,
    /// Physics kinematic body.
    pub physics_kinematic: bool,
    /// Events organized by type. Index by `event_type::*` constants.
    /// Each slot contains the sub-entries for that event type.
    pub events: Vec<Vec<EventEntry>>,
}

/// Parsed OBJT chunk.
#[derive(Debug)]
pub struct Objt {
    /// Object definitions.
    pub objects: Vec<ObjectEntry>,
}

impl Objt {
    /// Parse the OBJT chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `data` is the full file data (for following absolute pointers).
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut objects = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let obj = Self::parse_object(data, ptr as usize)?;
            objects.push(obj);
        }

        Ok(Self { objects })
    }

    fn parse_object(data: &[u8], offset: usize) -> Result<ObjectEntry> {
        let mut c = Cursor::new(data);
        c.seek(offset);

        let name = StringRef(c.read_u32()?);
        let sprite_index = c.read_i32()?;
        let visible = c.read_u32()? != 0;
        let solid = c.read_u32()? != 0;
        let depth = c.read_i32()?;
        let persistent = c.read_u32()? != 0;
        let parent_index = c.read_i32()?;
        let mask_index = c.read_i32()?;

        // Physics properties
        let physics_enabled = c.read_u32()? != 0;
        let physics_sensor = c.read_u32()? != 0;
        let physics_shape = PhysicsShape::from_u32(c.read_u32()?);
        let physics_density = c.read_f32()?;
        let physics_restitution = c.read_f32()?;
        let physics_group = c.read_u32()?;
        let physics_linear_damping = c.read_f32()?;
        let physics_angular_damping = c.read_f32()?;
        let vert_count = c.read_u32()? as usize;
        let physics_friction = c.read_f32()?;
        let physics_awake = c.read_u32()? != 0;
        let physics_kinematic = c.read_u32()? != 0;

        // Physics vertices (vert_count × 2 floats)
        let mut physics_vertices = Vec::with_capacity(vert_count);
        for _ in 0..vert_count {
            let x = c.read_f32()?;
            let y = c.read_f32()?;
            physics_vertices.push(PhysicsVertex { x, y });
        }

        // Event type lists
        let event_type_count = c.read_u32()? as usize;
        let mut event_ptrs = Vec::with_capacity(event_type_count);
        for _ in 0..event_type_count {
            event_ptrs.push(c.read_u32()?);
        }

        let mut events = Vec::with_capacity(event_type_count);
        for ptr in event_ptrs {
            let entries = Self::parse_event_list(data, ptr as usize)?;
            events.push(entries);
        }

        Ok(ObjectEntry {
            name,
            sprite_index,
            visible,
            solid,
            depth,
            persistent,
            parent_index,
            mask_index,
            physics_enabled,
            physics_sensor,
            physics_shape,
            physics_density,
            physics_restitution,
            physics_group,
            physics_linear_damping,
            physics_angular_damping,
            physics_vertices,
            physics_friction,
            physics_awake,
            physics_kinematic,
            events,
        })
    }

    fn parse_event_list(data: &[u8], offset: usize) -> Result<Vec<EventEntry>> {
        let mut c = Cursor::new(data);
        c.seek(offset);
        let pointers = c.read_pointer_list()?;

        let mut entries = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let entry = Self::parse_event_entry(data, ptr as usize)?;
            entries.push(entry);
        }

        Ok(entries)
    }

    fn parse_event_entry(data: &[u8], offset: usize) -> Result<EventEntry> {
        let mut c = Cursor::new(data);
        c.seek(offset);

        let subtype = c.read_u32()?;

        // Action list: count + count × pointers
        let action_ptrs = c.read_pointer_list()?;

        let mut actions = Vec::with_capacity(action_ptrs.len());
        for ptr in action_ptrs {
            let action = Self::parse_action(data, ptr as usize)?;
            actions.push(action);
        }

        Ok(EventEntry { subtype, actions })
    }

    fn parse_action(data: &[u8], offset: usize) -> Result<Action> {
        let mut c = Cursor::new(data);
        c.seek(offset);

        let lib_id = c.read_u32()?;
        let action_id = c.read_u32()?;
        let action_kind = c.read_u32()?;
        let has_relative = c.read_u32()? != 0;
        let is_question = c.read_u32()? != 0;
        let applies_to = c.read_i32()?;
        let exec_type = c.read_u32()?;
        let func_name = StringRef(c.read_u32()?);
        let code_id = c.read_u32()?;
        let arg_count = c.read_u32()?;
        let who = c.read_i32()?;
        let relative = c.read_u32()? != 0;
        let is_not = c.read_u32()? != 0;
        // Skip padding word
        c.skip(4)?;

        Ok(Action {
            lib_id,
            action_id,
            action_kind,
            has_relative,
            is_question,
            applies_to,
            exec_type,
            func_name,
            code_id,
            arg_count,
            who,
            relative,
            is_not,
        })
    }
}
