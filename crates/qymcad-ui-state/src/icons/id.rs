//! Strongly-typed CAD tool and action icon identifiers.

/// Logical identifier for an action or tool in the CAD interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IconId {
    // --- Sketch creation ---
    SketchSelect,
    SketchPoint,
    SketchLine,
    SketchRect,
    SketchCircle,
    SketchCircle3Pt,
    SketchArc,
    SketchPolygon,
    SketchSlot,
    SketchEllipse,
    SketchSpline,
    SketchText,
    SketchConstruction,

    // --- Sketch modification ---
    SketchFillet,
    SketchFilletAll,
    SketchChamfer,
    SketchTrim,
    SketchExtend,
    SketchBreak,
    SketchOffset,
    SketchMirror,
    SketchCopy,
    SketchArrayLinear,
    SketchArrayCircular,
    SketchDelete,
    SketchMove,
    SketchRotate,
    SketchMeasure,
    SketchProject,
    SketchDimLinear,
    SketchDimAngle,
    SketchDimRadius,

    // --- Sketch constraints ---
    ConstraintCoincident,
    ConstraintHorizontal,
    ConstraintVertical,
    ConstraintParallel,
    ConstraintPerpendicular,
    ConstraintEqual,
    ConstraintCollinear,
    ConstraintConcentric,
    ConstraintTangent,
    ConstraintSymmetric,
    ConstraintMidpoint,
    ConstraintLock,

    // --- Datums & Planes ---
    DatumPlane,
    DatumPoint,
    DatumAxis,
    SketchPickPlane,

    // --- 3D Solids ---
    PartExtrude,
    PartRevolve,
    PartSweep,
    PartLoft,
    PartFillet,
    PartChamfer,
    PartShell,
    PartHole,
    PartDraft,
    PartSection,
    PartThread,
    PartPushFace,
    PartRemoveFace,
    PartSplitBody,
    PartSplitFace,
    PartThicken,
    PartFaceCopy,
    PartMeasure,

    // --- 3D Primitives ---
    PartBox,
    PartCylinder,
    PartSphere,
    PartCone,
    PartTorus,
    PartPrism,

    // --- Booleans ---
    PartBooleanBodies,

    // --- Surfaces ---
    PartOffsetSurface,
    PartSurfaceReplace,
    PartPatch,
    PartStitch,
    PartTrimSurface,
    PartRecognise,

    // --- 3D Patterns ---
    PartArrayLinear,
    PartArrayCircular,
    PartMirror,

    // --- Assembly ---
    AssemblyNewPart,
    AssemblyNewSubassembly,
    AssemblyInsertComponent,
    AssemblyJoint,
    AssemblyGround,
    AssemblyGroup,
    AssemblyWidth,
    AssemblyTangent,
    AssemblyRelation,
    AssemblyArrayLinear,
    AssemblyArrayCircular,
}

include!(concat!(env!("OUT_DIR"), "/icon_generated.rs"));

/// Compatibility alias: CAD tools take strongly-typed `IconId`.
pub type IconSource = IconId;
