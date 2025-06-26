pub struct Framer {
    state: u8, // 0=wait for 0x94, 1=wait for 0xc3, etc.
    packet_len: u16,
    packet_buffer: [u8; 512],
    bytes_received: usize,
}

impl Framer {
    pub fn new() -> Self {
        Self {
            state: 0,
            packet_len: 0,
            packet_buffer: [0u8; 512],
            bytes_received: 0,
        }
    }

    /// Feed in new bytes. Returns Some(&[u8]) for each complete packet.
    pub fn push_bytes<'a>(&'a mut self, data: &[u8]) -> Option<&'a [u8]> {
        for &byte in data {
            match self.state {
                0 => {
                    if byte == 0x94 {
                        self.state = 1;
                    }
                }
                1 => {
                    if byte == 0xc3 {
                        self.state = 2;
                        self.packet_len = 0;
                    } else {
                        self.state = if byte == 0x94 { 1 } else { 0 };
                    }
                }
                2 => {
                    self.packet_len = (byte as u16) << 8;
                    self.state = 3;
                }
                3 => {
                    self.packet_len |= byte as u16;

                    if self.packet_len > 512 {
                        // Error: Invalid packet length
                        self.reset();
                        // Optionally, return an error
                        continue;
                    }

                    if self.packet_len == 0 {
                        // Zero-length, reset
                        self.reset();
                    } else {
                        self.bytes_received = 0;
                        self.state = 4;
                    }
                }
                4 => {
                    if self.bytes_received < self.packet_len as usize {
                        self.packet_buffer[self.bytes_received] = byte;
                        self.bytes_received += 1;
                    }
                    if self.bytes_received == self.packet_len as usize {
                        // Got a full packet! Store the packet length and reset state
                        let packet_len = self.packet_len as usize;
                        self.state = 0;
                        self.packet_len = 0;
                        self.bytes_received = 0;
                        return Some(&self.packet_buffer[..packet_len]); // yield one at a time
                    }
                }
                _ => {
                    self.reset();
                }
            }
        }
        None
    }

    fn reset(&mut self) {
        self.state = 0;
        self.packet_len = 0;
        self.bytes_received = 0;
        // not strictly necessary to clear buffer
    }
}
