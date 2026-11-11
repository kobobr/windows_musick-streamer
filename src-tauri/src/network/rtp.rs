pub const HEADER_LEN: usize = 12;
pub const PAYLOAD_TYPE_PCM_S16LE: u8 = 96;

pub fn build_packet(sequence: u16, timestamp: u32, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(HEADER_LEN + payload.len());
    packet.push(0x80);
    packet.push(PAYLOAD_TYPE_PCM_S16LE);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

pub fn payload(packet: &[u8]) -> Option<&[u8]> {
    if packet.len() < HEADER_LEN {
        return None;
    }
    if packet[0] >> 6 != 2 {
        return None;
    }
    Some(&packet[HEADER_LEN..])
}
