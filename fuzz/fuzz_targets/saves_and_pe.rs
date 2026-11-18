#![no_main]

use cmvs_core::format::{save, snapshot, system_save};
use cmvs_profile_research::exe;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = save::SaveHeader::parse(data);
    let _ = save::Csv2Layout::parse(data, data.len() as u64);
    let _ = save::Csv2Save::decode(data);
    if let Ok((_, decoded)) = save::Csv2Save::decode_any(data) {
        let _ = snapshot::VmSnapshot::decode(&decoded);
        let _ = snapshot::VisualSnapshot::decode(&decoded);
    }
    let _ = system_save::Css1Layout::parse(data, data.len() as u64);
    let _ = system_save::Css1Save::decode(data);

    let _ = snapshot::parse_records(data);
    let _ = snapshot::EngineSnapshot::decode(data);
    let _ = snapshot::CvnsSnapshot::decode(data);
    let _ = snapshot::VisualSnapshot::decode_records(data);
    let _ = snapshot::SavedEffect::decode(data);
    let _ = snapshot::LocalVariables::decode(data);

    if let Ok(pe) = exe::PeImage::parse(data) {
        let _ = pe.offset(0x1000, 16);
        let _ = exe::find_dispatch(&pe, data);
    }
});
