/** MMU Module (Contain all the non-graphic memory)

> From: Pan Docs - nocash / kOOPa
>
> General Memory Map
>
>  0000-3FFF   16KB ROM Bank 00     (in cartridge, fixed at bank 00)
>  4000-7FFF   16KB ROM Bank 01..NN (in cartridge, switchable bank number)
>  8000-9FFF   8KB Video RAM (VRAM) (switchable bank 0-1 in CGB Mode)
>  A000-BFFF   8KB External RAM     (in cartridge, switchable bank, if any)
>  C000-CFFF   4KB Work RAM Bank 0 (WRAM)
>  D000-DFFF   4KB Work RAM Bank 1 (WRAM)  (switchable bank 1-7 in CGB Mode)
>  E000-FDFF   Work RAM (Shadow) Same as C000-DDFF
>  FE00-FE9F   Sprite Attribute Table (OAM)
>  FEA0-FEFF   Not Usable
>  FF00-FF7F   I/O Ports
>  FF80-FFFE   High RAM (HRAM)
>  FFFF        Interrupt Enable Register
*/
use tools::*;
use vm::*;
use io;

/// Describe the divers interupt bits in the
/// interupt (e/f) Register.
#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub struct InterruptFlags {
    /// bit 0 : Vblank on/off
    pub vblank   : bool,
    /// bit 1 : LCD Stat on/off
    pub lcd_stat : bool,
    /// bit 2 : Timer on/off
    pub timer    : bool,
    /// bit 3 : Serial on/off
    pub serial   : bool,
    /// bit 4 : Joypad on/off
    pub joypad   : bool,
}

pub fn interrupt_to_u8(ir : InterruptFlags) -> u8 {
    return (ir.vblank as u8) << 0
        | (ir.lcd_stat as u8) << 1
        | (ir.timer as u8) << 2
        | (ir.serial as u8) << 3
        | (ir.joypad as u8) << 4;
}

pub fn u8_to_interrupt(byte : u8) -> InterruptFlags {
    return InterruptFlags {
        vblank   : (byte & 0x01) != 0,
        lcd_stat : (byte & 0x02) != 0,
        timer    : (byte & 0x04) != 0,
        serial   : (byte & 0x08) != 0,
        joypad   : (byte & 0x10) != 0,
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
/// The MMU (memory)
pub struct Mmu {
    /// GB Bios
    pub bios  : Vec<u8>,
    /// 0000-3FFF    16KB ROM Bank 00
    pub rom   : Vec<u8>,
    /// 4000-7FFF    16KB ROM Bank 01
    pub srom  : Vec<u8>,
    /// 8000-9FFF   Video RAM
    pub vram  : Vec<u8>,
    /// A000-BFFF    8KB External RAM
    pub eram  : Vec<u8>,
    /// C000-CFFF    4KB Work RAM Bank 0 (WRAM)
    pub wram  : Vec<u8>,
    /// D000-DFFF    4KB Work RAM Bank 1 (WRAM)
    pub swram : Vec<u8>,
    /// FE00-FE9F    Sprite Attribute Table (OAM)
    pub oam   : Vec<u8>,
    /// FF80-FFFE    High RAM (HRAM)
    pub hram  : Vec<u8>,
    /// FFFF         Interrupt Enable Register
    pub ier   : InterruptFlags,
    /// FF0F         Interrupt Flag Register
    pub ifr   : InterruptFlags,
    /// When true, reading below 0x100 access the bios.
    /// Once the booting sequence is finished, the value is
    /// turned to false. Then, rading below 0x100 read bytes from the rom field.
    pub bios_enabled : bool,

    /// JOYPAD register (P1)
    pub joyp  : u8,
}

impl Default for Mmu {
    fn default() -> Mmu { Mmu {
        bios : vec![
            0x31, 0xFE, 0xFF, 0xAF, 0x21, 0xFF, 0x9F, 0x32, 0xCB, 0x7C, 0x20, 0xFB, 0x21, 0x26, 0xFF, 0x0E,
            0x11, 0x3E, 0x80, 0x32, 0xE2, 0x0C, 0x3E, 0xF3, 0xE2, 0x32, 0x3E, 0x77, 0x77, 0x3E, 0xFC, 0xE0,
            0x47, 0x11, 0x04, 0x01, 0x21, 0x10, 0x80, 0x1A, 0xCD, 0x95, 0x00, 0xCD, 0x96, 0x00, 0x13, 0x7B,
            0xFE, 0x34, 0x20, 0xF3, 0x11, 0xD8, 0x00, 0x06, 0x08, 0x1A, 0x13, 0x22, 0x23, 0x05, 0x20, 0xF9,
            0x3E, 0x19, 0xEA, 0x10, 0x99, 0x21, 0x2F, 0x99, 0x0E, 0x0C, 0x3D, 0x28, 0x08, 0x32, 0x0D, 0x20,
            0xF9, 0x2E, 0x0F, 0x18, 0xF3, 0x67, 0x3E, 0x64, 0x57, 0xE0, 0x42, 0x3E, 0x91, 0xE0, 0x40, 0x04,
            0x1E, 0x02, 0x0E, 0x0C, 0xF0, 0x44, 0xFE, 0x90, 0x20, 0xFA, 0x0D, 0x20, 0xF7, 0x1D, 0x20, 0xF2,
            0x0E, 0x13, 0x24, 0x7C, 0x1E, 0x83, 0xFE, 0x62, 0x28, 0x06, 0x1E, 0xC1, 0xFE, 0x64, 0x20, 0x06,
            0x7B, 0xE2, 0x0C, 0x3E, 0x87, 0xF2, 0xF0, 0x42, 0x90, 0xE0, 0x42, 0x15, 0x20, 0xD2, 0x05, 0x20,
            0x4F, 0x16, 0x20, 0x18, 0xCB, 0x4F, 0x06, 0x04, 0xC5, 0xCB, 0x11, 0x17, 0xC1, 0xCB, 0x11, 0x17,
            0x05, 0x20, 0xF5, 0x22, 0x23, 0x22, 0x23, 0xC9, 0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B,
            0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E,
            0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC,
            0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E, 0x3c, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x4C,
            0x21, 0x04, 0x01, 0x11, 0xA8, 0x00, 0x1A, 0x13, 0xBE, 0x20, 0xFE, 0x23, 0x7D, 0xFE, 0x34, 0x20,
            0xF5, 0x06, 0x19, 0x78, 0x86, 0x23, 0x05, 0x20, 0xFB, 0x86, 0x20, 0xFE, 0x3E, 0x01, 0xE0, 0x50
        ],
        rom   : empty_memory(0x0000..0x4000),
        srom  : empty_memory(0x4000..0x8000),
        vram  : empty_memory(0x8000..0xF000),
        eram  : empty_memory(0xA000..0xC000),
        wram  : empty_memory(0xC000..0xD000),
        swram : empty_memory(0xD000..0xE000),
        oam   : empty_memory(0xFE00..0xFEA0),
        hram  : empty_memory(0xFF80..0xFFFF),
        ier   : Default::default(),
        ifr   : Default::default(),
        bios_enabled : true,

        joyp  : 0x3F,
    }
    }
}

/// Read a byte from MMU (TODO)
pub fn rb(addr : u16, vm : &Vm) -> u8 {
    let addr = addr as usize;
    let mmu = &vm.mmu;
    // TODO Check if memory (vram / OAM) is acessible
    // depending of the state of gpu.gpu_mode:GpuMode.
    match addr {
        0x0000...0x00FF => if mmu.bios_enabled {mmu.bios[addr]}
        else {
            mmu.rom[addr]
        },
        0x0100...0x3FFF => mmu.rom[addr],
        0x4000...0x7FFF => mmu.srom[addr - 0x4000],
        0x8000...0x9FFF => mmu.vram[addr - 0x8000],
        0xA000...0xBFFF => mmu.eram[addr - 0xA000],
        0xC000...0xCFFF => mmu.wram[addr - 0xC000],
        0xD000...0xDFFF => mmu.swram[addr - 0xD000],
        0xE000...0xEFFF => mmu.wram[addr - 0xE000],
        0xF000...0xFDFF => mmu.swram[addr - 0xF000],
        0xFE00...0xFE9F => mmu.oam[addr - 0xFE00],
        0xFF80...0xFFFE => mmu.hram[addr - 0xFF80],
        // Otherwise, it should be an IO
        _ => io::dispatch_io_read(addr, vm),
    }
}

/// Read a word (2 bytes) from MMU at address addr
pub fn rw(addr : u16, vm : &Vm) -> u16 {
    let l = rb(addr, vm);
    let h = rb(addr + 1, vm);
    w_combine(h, l)
}

static mut debug :u8 = 0;
/// Write a byte to the MMU at address addr (TODO)
pub fn wb(addr : u16, value : u8, vm : &mut Vm) {
    let addr = addr as usize;
    // TODO Check if memory (vram / OAM) is acessible
    // depending of the state of gpu.gpu_mode:GpuMode.
    match addr {
        0x0000...0x7FFF => return, // ROM is Read Only
        0x8000...0x9FFF => vm.mmu.vram[addr - 0x8000] = value,
        0xA000...0xBFFF => vm.mmu.eram[addr - 0xA000] = value,
        0xC000...0xCFFF => vm.mmu.wram[addr - 0xC000] = value,
        0xD000...0xDFFF => vm.mmu.swram[addr - 0xD000] = value,
        0xE000...0xEFFF => vm.mmu.wram[addr - 0xE000] = value,
        0xF000...0xFDFF => vm.mmu.swram[addr - 0xF000] = value,
        0xFE00...0xFE9F => {
            let index = addr - 0xFE00;
            vm.mmu.oam[index] = value;
            update_sprite(index, value, vm);
        },
        0xFF80...0xFFFE => vm.mmu.hram[addr - 0xFF80] = value,
        // Otherwise, it should be an IO
        _ => io::dispatch_io_write(addr, value, vm),
    }
    if addr == 0xFF01 {unsafe {
        debug = value;}
    }
    // Debug test roms
    if addr == 0xFF02 && value == 0x81 {unsafe {
        print!("{}", debug as char);
    }}

}

/// Write a word (2 bytes) into the MMU at adress addr
pub fn ww(addr : u16, value : u16, vm : &mut Vm) {
    let (h, l) = w_uncombine(value);
    wb(addr, l, vm);
    wb(addr + 1, h, vm);
}

/// Update the duplicated representation of a sprite
/// in GPU, used for sprite rendering.
pub fn update_sprite(index : usize, value : u8, vm : &mut Vm) {
    match index & 0x03 {
        0 => (*vm.gpu.sprites)[index / 4].y = (value as isize) - 16,
        1 => (*vm.gpu.sprites)[index / 4].x = (value as isize) - 8,
        2 => (*vm.gpu.sprites)[index / 4].tile_idx = value,
        3 => {
            (*vm.gpu.sprites)[index / 4].priority = (value & 0x80) == 0;
            (*vm.gpu.sprites)[index / 4].y_flip   = (value & 0x40) != 0;
            (*vm.gpu.sprites)[index / 4].x_flip   = (value & 0x20) != 0;
            (*vm.gpu.sprites)[index / 4].palette  = (value & 0x10) != 0;
        },
        // Impossible because of & 0x03:
        _ => return,
    }
}
