use jaytripper_core::ids::{SolarSystemId, StationId, StructureId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterLocation {
    pub solar_system_id: SolarSystemId,
    pub station_id: Option<StationId>,
    pub structure_id: Option<StructureId>,
}
