use vm::*;
use tools::*;
use gpu;
use mmu;
use std::boxed::Box;

//////////////////////////////////////////////////////////
// Registers and utilitary functions to manipulate them
//////////////////////////////////////////////////////////

#[derive(PartialEq, Eq, Debug)]
pub struct Registers {
        // Registers (a, b, c, d, e, h, l, f) :
        pub rs : [u8 ; 8],
        // Program counter
        pub pc : u16,
        // Stack pointer
        pub sp : u16,
}

impl Default for Registers {
    fn default() -> Registers {
        Registers {
            rs : [0x01, 0x00, 0x13, 0x00, 0xD8, 0x01, 0x4D, 0xB0],
            pc : 0x0000,
            sp : 0xFFFE,
        }
    }
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
/// Name of the register
pub enum Register {
    A = 0,
    B = 1,
    C = 2,
    D = 3,
    E = 4,
    H = 5,
    L = 6,
    F = 7,
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
/// List of flags
pub enum Flag {
    Z = 7,
    N = 6,
    H = 5,
    C = 4,
}

/// Macro for easy access to registers
///
/// Syntax : `reg![vm; register_name]`
/// where register_name : Register
#[macro_export]
macro_rules! reg {
    [$vm:expr ; $r:expr] => ($vm.cpu.registers.rs[$r as usize]);
}

/// Macro for accessing PC from a vm
///
/// Syntax : `pc![vm]`
#[macro_export]
macro_rules! pc {
    [$vm:expr] => ($vm.cpu.registers.pc);
}

/// Macro for acessing SP from a vm
///
/// Syntax : `sp![vm]`
#[macro_export]
macro_rules! sp {
    [$vm:expr] => ($vm.cpu.registers.sp);
}

/// Macro for acessing HL as a u16
/// (it's read only).
///
/// Syntax : `hl![vm]`
#[macro_export]
macro_rules! hl {
    [$vm:expr] => (
        w_combine(reg![$vm ; Register::H],
                  reg![$vm ; Register::L]
        )
    );
}

/// Macro for acessing HL as a u16
/// (it's read only).
///
/// Syntax : `hl![vm]`
#[macro_export]
macro_rules! flag {
    [$vm:expr ; $flag:expr] => {{
        0x01 & reg![$vm ; Register::F] >> ($flag as usize) == 0x01
    }}
}

/// Macro for setting a u16 value into the register h:l
/// (the juxtaposition of the two registers)
macro_rules! set_hl {
    ($vm:expr, $value:expr) => {{
        let (h, l) = w_uncombine($value as u16);
        reg![$vm ; Register::H] = h;
        reg![$vm ; Register::L] = l;
    }}
}

/// Reset the flags of the Vm (set all flags to 0)
pub fn reset_flags(vm: &mut Vm) {
    reg![vm ; Register::F] = 0
}

/// Set the specified flag to the value given
pub fn set_flag(vm : &mut Vm, flag : Flag, value : bool) {
    if value {
        reg![vm ; Register::F] |= 1 << flag as usize
    }
    else {
        reg![vm ; Register::F] &= !(1 << flag as usize)
    }
}

/// Get the value from two registers h and l glued together (h:l)
pub fn get_r16(vm : &mut Vm, h : Register, l : Register) -> u16 {
    let initial_h = reg![vm ; h];
    let initial_l = reg![vm ; l];
    w_combine(initial_h, initial_l)
}

/// Set the value of two registers h and l glued together (h:l)
pub fn set_r16(vm : &mut Vm, h : Register, l : Register, value : u16) {
    let (value_h, value_l) = w_uncombine(value);
    reg![vm ; h] = value_h;
    reg![vm ; l] = value_l;
    if l == Register::F {
        reg![vm ; l] &= 0xF0;
    }
}

//////////////////////////////////////////
// CPU structurs, data types, and states
//////////////////////////////////////////

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
/// Represent a 'time' enlapsed
pub struct Clock {
    /// Length in byte of the last instruction
    pub m : u64,
    /// Duration in cycles
    pub t : u64,
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub struct Timers {
    /// DIV Divider Register : incremented each 4 cyles
    pub div : u8,
    /// TIMA Timer counter : timer incremented each n-cycles (see TAC)
    pub tima : u8,
    /// TMA Timer Modulo : reset value for TIMA when TIMA overflow.
    pub tma : u8,
    /// TAC Timer Control : timer control register. Settings for TIMA.
    pub tac : TimerControl,

    //// IMPLEMENTATION

    /// This timer over each 4 cycles
    pub imp_4c : u64,
    /// This timer overflow each n-cycles (n is controled by tac)
    pub imp_nc : u64,
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub struct TimerControl {
    /// Input Clock Selector
    /// 00 : 16 cycles  [  4096Hz]
    /// 01 : 1 cycle    [262144Hz]
    /// 10 : 8 cycles   [ 65536Hz]
    /// 11 : 4 cycles   [ 16384Hz]
    timer_mode : u8,
    /// Timer Stop
    /// 0 : Stop Timer
    /// 1 : Start Timer
    running : bool,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum InterruptState {
    IEnabled,
    IDisabled,
    IDisableNextInst,
    IEnableNextInst,
}

impl Default for InterruptState {
    fn default() -> InterruptState { InterruptState::IDisabled }
}

#[derive(PartialEq, Eq, Default, Debug)]
pub struct Cpu {
    /// CPU's registers
    pub registers : Registers,
    /// Time in cycle enlapsed since the first instruction
    pub clock : Clock,
    /// Act as an enhanced IME register
    /// IME = Interupt Master Enable
    pub interrupt : InterruptState,

    /// Timer implementation
    pub timers : Timers,
}

/// Read a byte from the memory pointed by PC, and increment PC
pub fn read_program_byte(vm : &mut Vm) -> u8 {
    let byte = mmu::rb(pc![vm], vm);
    pc![vm] = pc![vm].wrapping_add(1);
    return byte;
}

/// Read a word (2bytes) from the memory pointed by PC, and increment PC
pub fn read_program_word(vm : &mut Vm) -> u16 {
    let word = mmu::rw(pc![vm], vm);
    pc![vm] = pc![vm].wrapping_add(2);
    return word;
}

/// Store a CPU's instruction, that is a string describing the assembly instruction, and the *function pointer*
pub struct Instruction(&'static str, Box<Fn(&mut Vm) -> Clock>);

/// Add the values of clock into the cpu's clock
pub fn update_cpu_clock(clock : Clock, vm : &mut Vm) {
    vm.cpu.clock.m = vm.cpu.clock.m.wrapping_add(clock.m);
    vm.cpu.clock.t = vm.cpu.clock.t.wrapping_add(clock.t);
}

/// Update timers with the enlapsed time clock
pub fn update_timers(clock : Clock, vm : &mut Vm) {
    let t = &mut vm.cpu.timers;
    let ifr = &mut vm.mmu.ifr;

    // Handle DIV timer
    t.imp_4c += clock.t;
    while t.imp_4c >= 4 {
        t.imp_4c -= 4;
        t.div = t.div.wrapping_add(1);
    }

    // Handle TIMA timer
    if t.tac.running {
        // Check the time step depending on mode
        let diff = match t.tac.timer_mode {
            0b00 => 16,
            0b01 => 1,
            0b10 => 8,
            0b11 => 4,
            _    => {
                println!("Timer Mode equal to {} where value in [0,3] expected!",
                t.tac.timer_mode);
                16
            },
        };

        t.imp_nc += clock.t;
        // Take into account each time step
        while t.imp_nc >= diff {
            t.imp_nc -= diff;

            // If the counter is about to overflow
            if t.tima == 0xFF {
                // Reset timer and set interrupt flag
                t.tima = t.tma;
                ifr.timer = true;
            } else {
                // Increment timer
                t.tima = t.tima.wrapping_add(1);
            }
        }
    }
}

/// Execute exactly one instruction by the CPU
///
/// The function load the byte pointed by PC, increment PC,
/// and call dispatch with the opcode to run the instruction.
pub fn execute_one_instruction(vm : &mut Vm) {
    // Disable bios if needed
    if pc![vm] >= 0x100 {
        vm.mmu.bios_enabled = false;
    }

    //print!("0x{:04x}:", pc![vm]);
    //let old_pc = pc![vm];

    // Run the instruction
    let opcode = read_program_byte(vm);
    let Instruction(name, fct) = match opcode {
        0xCB => dispatch_cb(read_program_byte(vm)),
        _    => dispatch(opcode),
    };

    // Debug :
/*    println!(":{:04X}|{}\tSP:{:02X} AF:{:02X}{:02X} BC:{:02X}{:02X} DE:{:02X}{:02X} HL:{:02X}{:02X} LY:{:02X}",
             old_pc,
             name, sp![vm],
             reg![vm ; Register::A], reg![vm ; Register::F],
             reg![vm ; Register::B], reg![vm ; Register::C],
             reg![vm ; Register::D], reg![vm ; Register::E],
             reg![vm ; Register::H], reg![vm ; Register::L],
             vm.gpu.line,
    );*/

    // Run opcode
    let clock = (fct)(vm);

    // Update CPU's clock and timers
    update_cpu_clock(clock, vm);
    update_timers(clock, vm);

    // Handle interupts
    if vm.cpu.interrupt == InterruptState::IDisableNextInst
        || vm.cpu.interrupt == InterruptState::IEnabled {
        let clock = handle_interrupts(vm);

        // Update CPU's clock and timers
        update_cpu_clock(clock, vm);
        update_timers(clock, vm);
    }

    // Update the interrupt state
    vm.cpu.interrupt = match vm.cpu.interrupt {
        InterruptState::IEnableNextInst =>  InterruptState::IEnabled,
        InterruptState::IDisableNextInst => InterruptState::IDisabled,
        _ => vm.cpu.interrupt,
    };


    // Update GPU's mode (Clock, Scanline, VBlank, HBlank, ...)
    gpu::update_gpu_mode(vm, clock.t);
}

pub fn handle_interrupts(vm : &mut Vm) -> Clock {
    // Handle vblank
    if vm.mmu.ier.vblank && vm.mmu.ifr.vblank {
        vm.mmu.ifr.vblank = false;
        vm.cpu.interrupt = InterruptState::IDisabled;
        return i_rst(vm, 0x40);
    }
    if vm.mmu.ier.lcd_stat && vm.mmu.ifr.lcd_stat {
        vm.mmu.ifr.lcd_stat = false;
        vm.cpu.interrupt = InterruptState::IDisabled;
        return i_rst(vm, 0x48);
    }
    if vm.mmu.ier.timer && vm.mmu.ifr.timer {
        vm.mmu.ifr.timer = false;
        vm.cpu.interrupt = InterruptState::IDisabled;
        return i_rst(vm, 0x50);
    }
    if vm.mmu.ier.serial && vm.mmu.ifr.serial {
        vm.mmu.ifr.serial = false;
        vm.cpu.interrupt = InterruptState::IDisabled;
        return i_rst(vm, 0x58);
    }
    if vm.mmu.ier.joypad && vm.mmu.ifr.joypad {
        vm.mmu.ifr.joypad = false;
        vm.cpu.interrupt = InterruptState::IDisabled;
        return i_rst(vm, 0x60);
    }
    return Clock { m:0, t:0 };
}

/// Simple macro for writing dispatch more easily
macro_rules! mk_inst {
    [$vm:ident > $name:expr , $f:expr] => {{
        Instruction($name, Box::new(|$vm : &mut Vm| $f))
    }}
}

/// Associate to each opcode:u8 it's instruction:Instruction
pub fn dispatch(opcode : u8) -> Instruction {
    match opcode {
        0x00 => mk_inst![vm> "NOP",     i_nop(vm)],
        0x01 => mk_inst![vm> "LDBCd16", i_ldr16d16(vm, Register::B, Register::C)],
        0x02 => mk_inst![vm> "LDBCmA",  i_ldr16mr(vm, Register::B, Register::C, Register::A)],
        0x03 => mk_inst![vm> "INCBC",   i_incr16(vm, Register::B, Register::C)],
        0x04 => mk_inst![vm> "INCB",    i_incr(vm, Register::B)],
        0x05 => mk_inst![vm> "DECB",    i_decr(vm, Register::B)],
        0x06 => mk_inst![vm> "LDBd8",   i_ldrd8(vm, Register::B)],
        0x07 => mk_inst![vm> "RLCA",    i_rlca(vm)],
        0x08 => mk_inst![vm> "LDa16mSP",i_lda16msp(vm)],
        0x09 => mk_inst![vm> "ADDHLBC", i_addhlr16(vm, Register::B, Register::C)],
        0x0A => mk_inst![vm> "LDABCm",  i_ldrr16m(vm, Register::A, Register::B, Register::C)],
        0x0B => mk_inst![vm> "DECBC",   i_decr16(vm, Register::B, Register::C)],
        0x0C => mk_inst![vm> "INCC",    i_incr(vm, Register::C)],
        0x0D => mk_inst![vm> "DECC",    i_decr(vm, Register::C)],
        0x0E => mk_inst![vm> "LDCd8",   i_ldrd8(vm, Register::C)],
        0x0F => mk_inst![vm> "RRCA",    i_rrca(vm)],

        //0x10 => STOP
        0x10 => mk_inst![vm> "STOP",    i_nop(vm)],
        0x11 => mk_inst![vm> "LDDEd16", i_ldr16d16(vm, Register::D, Register::E)],
        0x12 => mk_inst![vm> "LDDEmA",  i_ldr16mr(vm, Register::D, Register::E, Register::A)],
        0x13 => mk_inst![vm> "INCDE",   i_incr16(vm, Register::D, Register::E)],
        0x14 => mk_inst![vm> "INCD",    i_incr(vm, Register::D)],
        0x15 => mk_inst![vm> "DECD",    i_decr(vm, Register::D)],
        0x16 => mk_inst![vm> "LDDd8",   i_ldrd8(vm, Register::D)],
        0x17 => mk_inst![vm> "RLA",     i_rla(vm)],
        0x18 => mk_inst![vm> "JR",      i_jr(vm)],
        0x19 => mk_inst![vm> "ADDHLDE", i_addhlr16(vm, Register::D, Register::E)],
        0x1A => mk_inst![vm> "LDADEm",  i_ldrr16m(vm, Register::A, Register::D, Register::E)],
        0x1B => mk_inst![vm> "DECDE",   i_decr16(vm, Register::D, Register::E)],
        0x1C => mk_inst![vm> "INCE",    i_incr(vm, Register::E)],
        0x1D => mk_inst![vm> "DECE",    i_decr(vm, Register::E)],
        0x1E => mk_inst![vm> "LDEd8",   i_ldrd8(vm, Register::E)],
        0x1F => mk_inst![vm> "RRA",     i_rra(vm)],

        0x20 => mk_inst![vm> "JRnfZ",   i_jrnf(vm, Flag::Z)],
        0x21 => mk_inst![vm> "LDHLd16", i_ldr16d16(vm, Register::H, Register::L)],
        0x22 => mk_inst![vm> "LDIHLmA", i_ldihlma(vm)],
        0x23 => mk_inst![vm> "INCHL",   i_incr16(vm, Register::H, Register::L)],
        0x24 => mk_inst![vm> "INCH",    i_incr(vm, Register::H)],
        0x25 => mk_inst![vm> "DECH",    i_decr(vm, Register::H)],
        0x26 => mk_inst![vm> "LDHd8",   i_ldrd8(vm, Register::H)],
        0x27 => mk_inst![vm> "DAA",     i_daa(vm)],
        0x28 => mk_inst![vm> "JRfZ",    i_jrf(vm, Flag::Z)],
        0x29 => mk_inst![vm> "ADDHLHL", i_addhlr16(vm, Register::H, Register::L)],
        0x2A => mk_inst![vm> "LDIAHLm", i_ldiahlm(vm)],
        0x2B => mk_inst![vm> "DECHL",   i_decr16(vm, Register::H, Register::L)],
        0x2C => mk_inst![vm> "INCL",    i_incr(vm, Register::L)],
        0x2D => mk_inst![vm> "DECL",    i_decr(vm, Register::L)],
        0x2E => mk_inst![vm> "LDLd8",   i_ldrd8(vm, Register::L)],
        0x2F => mk_inst![vm> "CPL",     i_cpl(vm)],

        0x30 => mk_inst![vm> "JRnfC",   i_jrnf(vm, Flag::C)],
        0x31 => mk_inst![vm> "LDSPd16", i_ldspd16(vm)],
        0x32 => mk_inst![vm> "LDDHLmA", i_lddhlma(vm)],
        0x33 => mk_inst![vm> "INSP",    i_incsp(vm)],
        0x34 => mk_inst![vm> "INHLm",   i_inchlm(vm)],
        0x35 => mk_inst![vm> "DECHLm",  i_dechlm(vm)],
        0x36 => mk_inst![vm> "LDHLmd8", i_ldhlmd8(vm)],
        0x37 => mk_inst![vm> "SCF",     i_scf(vm)],
        0x38 => mk_inst![vm> "JRfZ",    i_jrf(vm, Flag::C)],
        0x39 => mk_inst![vm> "ADDHLSP", i_addhlsp(vm)],
        0x3A => mk_inst![vm> "LDDAHLm", i_lddahlm(vm)],
        0x3B => mk_inst![vm> "DECSP",   i_decsp(vm)],
        0x3C => mk_inst![vm> "INCA",    i_incr(vm, Register::A)],
        0x3D => mk_inst![vm> "DECA",    i_decr(vm, Register::A)],
        0x3E => mk_inst![vm> "LDAd8",   i_ldrd8(vm, Register::A)],
        0x3F => mk_inst![vm> "CCF",     i_ccf(vm)],

        0x40 => mk_inst![vm> "LDBB",    i_ldrr(vm, Register::B, Register::B)],
        0x41 => mk_inst![vm> "LDBC",    i_ldrr(vm, Register::B, Register::C)],
        0x42 => mk_inst![vm> "LDBD",    i_ldrr(vm, Register::B, Register::D)],
        0x43 => mk_inst![vm> "LDBE",    i_ldrr(vm, Register::B, Register::E)],
        0x44 => mk_inst![vm> "LDBH",    i_ldrr(vm, Register::B, Register::H)],
        0x45 => mk_inst![vm> "LDBL",    i_ldrr(vm, Register::B, Register::L)],
        0x46 => mk_inst![vm> "LDBHLm",  i_ldrr16m(vm, Register::B, Register::H, Register::L)],
        0x47 => mk_inst![vm> "LDBA",    i_ldrr(vm, Register::B, Register::A)],
        0x48 => mk_inst![vm> "LDCB",    i_ldrr(vm, Register::C, Register::B)],
        0x49 => mk_inst![vm> "LDCC",    i_ldrr(vm, Register::C, Register::C)],
        0x4A => mk_inst![vm> "LDCD",    i_ldrr(vm, Register::C, Register::D)],
        0x4B => mk_inst![vm> "LDCE",    i_ldrr(vm, Register::C, Register::E)],
        0x4C => mk_inst![vm> "LDCH",    i_ldrr(vm, Register::C, Register::H)],
        0x4D => mk_inst![vm> "LDCL",    i_ldrr(vm, Register::C, Register::L)],
        0x4E => mk_inst![vm> "LDCHLm",  i_ldrr16m(vm, Register::C, Register::H, Register::L)],
        0x4F => mk_inst![vm> "LDCA",    i_ldrr(vm, Register::C, Register::A)],

        0x50 => mk_inst![vm> "LDDB",    i_ldrr(vm, Register::D, Register::B)],
        0x51 => mk_inst![vm> "LDDC",    i_ldrr(vm, Register::D, Register::C)],
        0x52 => mk_inst![vm> "LDDD",    i_ldrr(vm, Register::D, Register::D)],
        0x53 => mk_inst![vm> "LDDE",    i_ldrr(vm, Register::D, Register::E)],
        0x54 => mk_inst![vm> "LDDH",    i_ldrr(vm, Register::D, Register::H)],
        0x55 => mk_inst![vm> "LDDL",    i_ldrr(vm, Register::D, Register::L)],
        0x56 => mk_inst![vm> "LDDHLm",  i_ldrr16m(vm, Register::D, Register::H, Register::L)],
        0x57 => mk_inst![vm> "LDDA",    i_ldrr(vm, Register::D, Register::A)],
        0x58 => mk_inst![vm> "LDEB",    i_ldrr(vm, Register::E, Register::B)],
        0x59 => mk_inst![vm> "LDEC",    i_ldrr(vm, Register::E, Register::C)],
        0x5A => mk_inst![vm> "LDED",    i_ldrr(vm, Register::E, Register::D)],
        0x5B => mk_inst![vm> "LDEE",    i_ldrr(vm, Register::E, Register::E)],
        0x5C => mk_inst![vm> "LDEH",    i_ldrr(vm, Register::E, Register::H)],
        0x5D => mk_inst![vm> "LDEL",    i_ldrr(vm, Register::E, Register::L)],
        0x5E => mk_inst![vm> "LDEHLm",  i_ldrr16m(vm, Register::E, Register::H, Register::L)],
        0x5F => mk_inst![vm> "LDEA",    i_ldrr(vm, Register::E, Register::A)],

        0x60 => mk_inst![vm> "LDHB",    i_ldrr(vm, Register::H, Register::B)],
        0x61 => mk_inst![vm> "LDHC",    i_ldrr(vm, Register::H, Register::C)],
        0x62 => mk_inst![vm> "LDHD",    i_ldrr(vm, Register::H, Register::D)],
        0x63 => mk_inst![vm> "LDHE",    i_ldrr(vm, Register::H, Register::E)],
        0x64 => mk_inst![vm> "LDHH",    i_ldrr(vm, Register::H, Register::H)],
        0x65 => mk_inst![vm> "LDHL",    i_ldrr(vm, Register::H, Register::L)],
        0x66 => mk_inst![vm> "LDHHLm",  i_ldrr16m(vm, Register::H, Register::H, Register::L)],
        0x67 => mk_inst![vm> "LDHA",    i_ldrr(vm, Register::H, Register::A)],
        0x68 => mk_inst![vm> "LDLB",    i_ldrr(vm, Register::L, Register::B)],
        0x69 => mk_inst![vm> "LDLC",    i_ldrr(vm, Register::L, Register::C)],
        0x6A => mk_inst![vm> "LDLD",    i_ldrr(vm, Register::L, Register::D)],
        0x6B => mk_inst![vm> "LDLE",    i_ldrr(vm, Register::L, Register::E)],
        0x6C => mk_inst![vm> "LDLH",    i_ldrr(vm, Register::L, Register::H)],
        0x6D => mk_inst![vm> "LDLL",    i_ldrr(vm, Register::L, Register::L)],
        0x6E => mk_inst![vm> "LDLHLm",  i_ldrr16m(vm, Register::L, Register::H, Register::L)],
        0x6F => mk_inst![vm> "LDLA",    i_ldrr(vm, Register::L, Register::A)],

        0x70 => mk_inst![vm> "LDHLmB",  i_ldr16mr(vm, Register::H, Register::L, Register::B)],
        0x71 => mk_inst![vm> "LDHLmC",  i_ldr16mr(vm, Register::H, Register::L, Register::C)],
        0x72 => mk_inst![vm> "LDHLmD",  i_ldr16mr(vm, Register::H, Register::L, Register::D)],
        0x73 => mk_inst![vm> "LDHLmE",  i_ldr16mr(vm, Register::H, Register::L, Register::E)],
        0x74 => mk_inst![vm> "LDHLmH",  i_ldr16mr(vm, Register::H, Register::L, Register::H)],
        0x75 => mk_inst![vm> "LDHLmL",  i_ldr16mr(vm, Register::H, Register::L, Register::L)],
        0x76 => mk_inst![vm> "HALT",    Default::default()],
        0x77 => mk_inst![vm> "LDHLmA",  i_ldr16mr(vm, Register::H, Register::L, Register::A)],
        0x78 => mk_inst![vm> "LDAB",    i_ldrr(vm, Register::A, Register::B)],
        0x79 => mk_inst![vm> "LDAC",    i_ldrr(vm, Register::A, Register::C)],
        0x7A => mk_inst![vm> "LDAD",    i_ldrr(vm, Register::A, Register::D)],
        0x7B => mk_inst![vm> "LDAE",    i_ldrr(vm, Register::A, Register::E)],
        0x7C => mk_inst![vm> "LDAH",    i_ldrr(vm, Register::A, Register::H)],
        0x7D => mk_inst![vm> "LDAL",    i_ldrr(vm, Register::A, Register::L)],
        0x7E => mk_inst![vm> "LDAHLm",  i_ldrr16m(vm, Register::A, Register::H, Register::L)],
        0x7F => mk_inst![vm> "LDAA",    i_ldrr(vm, Register::A, Register::A)],

        0x80 => mk_inst![vm> "ADDB",    i_addr(vm, Register::B)],
        0x81 => mk_inst![vm> "ADDC",    i_addr(vm, Register::C)],
        0x82 => mk_inst![vm> "ADDD",    i_addr(vm, Register::D)],
        0x83 => mk_inst![vm> "ADDE",    i_addr(vm, Register::E)],
        0x84 => mk_inst![vm> "ADDH",    i_addr(vm, Register::H)],
        0x85 => mk_inst![vm> "ADDL",    i_addr(vm, Register::L)],
        0x86 => mk_inst![vm> "ADDHLm",  i_addhlm(vm)],
        0x87 => mk_inst![vm> "ADDA",    i_addr(vm, Register::A)],
        0x88 => mk_inst![vm> "ADCB",    i_adcr(vm, Register::B)],
        0x89 => mk_inst![vm> "ADCC",    i_adcr(vm, Register::C)],
        0x8A => mk_inst![vm> "ADCD",    i_adcr(vm, Register::D)],
        0x8B => mk_inst![vm> "ADCE",    i_adcr(vm, Register::E)],
        0x8C => mk_inst![vm> "ADCH",    i_adcr(vm, Register::H)],
        0x8D => mk_inst![vm> "ADCL",    i_adcr(vm, Register::L)],
        0x8E => mk_inst![vm> "ADCHLm",  i_adchlm(vm)],
        0x8F => mk_inst![vm> "ADCA",    i_adcr(vm, Register::A)],

        0x90 => mk_inst![vm> "SUBB",    i_subr(vm, Register::B)],
        0x91 => mk_inst![vm> "SUBC",    i_subr(vm, Register::C)],
        0x92 => mk_inst![vm> "SUBD",    i_subr(vm, Register::D)],
        0x93 => mk_inst![vm> "SUBE",    i_subr(vm, Register::E)],
        0x94 => mk_inst![vm> "SUBH",    i_subr(vm, Register::H)],
        0x95 => mk_inst![vm> "SUBL",    i_subr(vm, Register::L)],
        0x96 => mk_inst![vm> "SUBHLm",  i_subhlm(vm)],
        0x97 => mk_inst![vm> "SUBA",    i_subr(vm, Register::A)],
        0x98 => mk_inst![vm> "SBCB",    i_sbcr(vm, Register::B)],
        0x99 => mk_inst![vm> "SBCC",    i_sbcr(vm, Register::C)],
        0x9A => mk_inst![vm> "SBCD",    i_sbcr(vm, Register::D)],
        0x9B => mk_inst![vm> "SBCE",    i_sbcr(vm, Register::E)],
        0x9C => mk_inst![vm> "SBCH",    i_sbcr(vm, Register::H)],
        0x9D => mk_inst![vm> "SBCL",    i_sbcr(vm, Register::L)],
        0x9E => mk_inst![vm> "SBCHLm",  i_sbchlm(vm)],
        0x9F => mk_inst![vm> "SBCA",    i_sbcr(vm, Register::A)],

        0xA0 => mk_inst![vm> "ANDB",    i_andr(vm, Register::B)],
        0xA1 => mk_inst![vm> "ANDC",    i_andr(vm, Register::C)],
        0xA2 => mk_inst![vm> "ANDD",    i_andr(vm, Register::D)],
        0xA3 => mk_inst![vm> "ANDE",    i_andr(vm, Register::E)],
        0xA4 => mk_inst![vm> "ANDH",    i_andr(vm, Register::H)],
        0xA5 => mk_inst![vm> "ANDL",    i_andr(vm, Register::L)],
        0xA6 => mk_inst![vm> "ANDHLm",  i_andhlm(vm)],
        0xA7 => mk_inst![vm> "ANDA",    i_andr(vm, Register::A)],
        0xA8 => mk_inst![vm> "XORB",    i_xorr(vm, Register::B)],
        0xA9 => mk_inst![vm> "XORC",    i_xorr(vm, Register::C)],
        0xAA => mk_inst![vm> "XORD",    i_xorr(vm, Register::D)],
        0xAB => mk_inst![vm> "XORE",    i_xorr(vm, Register::E)],
        0xAC => mk_inst![vm> "XORH",    i_xorr(vm, Register::H)],
        0xAD => mk_inst![vm> "XORL",    i_xorr(vm, Register::L)],
        0xAE => mk_inst![vm> "XORHLm",  i_xorhlm(vm)],
        0xAF => mk_inst![vm> "XORA",    i_xorr(vm, Register::A)],

        0xB0 => mk_inst![vm> "ORB",     i_orr(vm, Register::B)],
        0xB1 => mk_inst![vm> "ORC",     i_orr(vm, Register::C)],
        0xB2 => mk_inst![vm> "ORD",     i_orr(vm, Register::D)],
        0xB3 => mk_inst![vm> "ORE",     i_orr(vm, Register::E)],
        0xB4 => mk_inst![vm> "ORH",     i_orr(vm, Register::H)],
        0xB5 => mk_inst![vm> "ORL",     i_orr(vm, Register::L)],
        0xB6 => mk_inst![vm> "ORHLm",   i_orhlm(vm)],
        0xB7 => mk_inst![vm> "ORA",     i_orr(vm, Register::A)],
        0xB8 => mk_inst![vm> "CPB",     i_cpr(vm, Register::B)],
        0xB9 => mk_inst![vm> "CPC",     i_cpr(vm, Register::C)],
        0xBA => mk_inst![vm> "CPD",     i_cpr(vm, Register::D)],
        0xBB => mk_inst![vm> "CPE",     i_cpr(vm, Register::E)],
        0xBC => mk_inst![vm> "CPH",     i_cpr(vm, Register::H)],
        0xBD => mk_inst![vm> "CPL",     i_cpr(vm, Register::L)],
        0xBE => mk_inst![vm> "CPHLm",   i_cphlm(vm)],
        0xBF => mk_inst![vm> "CPA",     i_cpr(vm, Register::A)],

        0xC0 => mk_inst![vm> "RETNZ",   i_retnf(vm, Flag::Z)],
        0xC1 => mk_inst![vm> "POPBC",   i_pop(vm, Register::B, Register::C)],
        0xC2 => mk_inst![vm> "JPnfZ",   i_jpnf(vm, Flag::Z)],
        0xC3 => mk_inst![vm> "JP",      i_jp(vm)],
        0xC4 => mk_inst![vm> "CALLnZ",  i_callnf(vm, Flag::Z)],
        0xC5 => mk_inst![vm> "PUSHBC",  i_push(vm, Register::B, Register::C)],
        0xC6 => mk_inst![vm> "ADDd8",   i_addd8(vm)],
        0xC7 => mk_inst![vm> "RST00h",  i_rst(vm, 0x00)],
        0xC8 => mk_inst![vm> "RETZ",    i_retf(vm, Flag::Z)],
        0xC9 => mk_inst![vm> "RET",     i_ret(vm)],
        0xCA => mk_inst![vm> "JPfZ",    i_jpf(vm, Flag::Z)],
        0xCB => Instruction("CBPref", Box::new(|_ : &mut Vm| Clock { m:0, t:0 })),
        0xCC => mk_inst![vm> "CALLZ",   i_callf(vm, Flag::Z)],
        0xCD => mk_inst![vm> "CALL",    i_call(vm)],
        0xCE => mk_inst![vm> "ADCd8",   i_adcd8(vm)],
        0xCF => mk_inst![vm> "RST08h",  i_rst(vm, 0x08)],

        0xD0 => mk_inst![vm> "RETNC",   i_retnf(vm, Flag::C)],
        0xD1 => mk_inst![vm> "POPDE",   i_pop(vm, Register::D, Register::E)],
        0xD2 => mk_inst![vm> "JPnfC",   i_jpnf(vm, Flag::C)],
        0xD3 => mk_inst![vm> "0xD3",    i_invalid(vm, 0xD3)],
        0xD4 => mk_inst![vm> "CALLnC",  i_callnf(vm, Flag::C)],
        0xD5 => mk_inst![vm> "PUSHDE",  i_push(vm, Register::D, Register::E)],
        0xD6 => mk_inst![vm> "SUBd8",   i_subd8(vm)],
        0xD7 => mk_inst![vm> "RST10h",  i_rst(vm, 0x10)],
        0xD8 => mk_inst![vm> "RETC",    i_retf(vm, Flag::C)],
        0xD9 => mk_inst![vm> "RETI",    i_reti(vm)],
        0xDA => mk_inst![vm> "JPfC",    i_jpf(vm, Flag::C)],
        0xDB => mk_inst![vm> "0xDB",    i_invalid(vm, 0xDB)],
        0xDC => mk_inst![vm> "CALLC",   i_callf(vm, Flag::C)],
        0xDD => mk_inst![vm> "0xDD",    i_invalid(vm, 0xDD)],
        0xDE => mk_inst![vm> "SBCd8",   i_sbcd8(vm)],
        0xDF => mk_inst![vm> "RST18h",  i_rst(vm, 0x18)],

        0xE0 => mk_inst![vm> "LDHa8mA", i_ldha8ma(vm)],
        0xE1 => mk_inst![vm> "POPHL",   i_pop(vm, Register::H, Register::L)],
        0xE2 => mk_inst![vm> "LDCmA",   i_ldcma(vm)],
        0xE3 => mk_inst![vm> "0xE3",    i_invalid(vm, 0xE3)],
        0xE4 => mk_inst![vm> "0xD3",    i_invalid(vm, 0xE4)],
        0xE5 => mk_inst![vm> "PUSHHL",  i_push(vm, Register::H, Register::L)],
        0xE6 => mk_inst![vm> "ANDd8",   i_andd8(vm)],
        0xE7 => mk_inst![vm> "RST20h",  i_rst(vm, 0x20)],
        0xE8 => mk_inst![vm> "ADDSPr8", i_addspr8(vm)],
        0xE9 => mk_inst![vm> "JPHL",    i_jphl(vm)],
        0xEA => mk_inst![vm> "LDa16mA", i_lda16ma(vm)],
        0xEB => mk_inst![vm> "0xEB",    i_invalid(vm, 0xEB)],
        0xEC => mk_inst![vm> "0xEC",    i_invalid(vm, 0xEC)],
        0xED => mk_inst![vm> "0xED",    i_invalid(vm, 0xED)],
        0xEE => mk_inst![vm> "XORd8",   i_xord8(vm)],
        0xEF => mk_inst![vm> "RST28h",  i_rst(vm, 0x28)],

        0xF0 => mk_inst![vm> "LDHAa8m", i_ldhaa8m(vm)],
        0xF1 => mk_inst![vm> "POPAF",   i_pop(vm, Register::A, Register::F)],
        0xF2 => mk_inst![vm> "LDACm",   i_ldacm(vm)],
        0xF3 => mk_inst![vm> "DI",      i_di(vm)],
        0xF4 => mk_inst![vm> "0xF4",    i_invalid(vm, 0xF4)],
        0xF5 => mk_inst![vm> "PUSHAF",  i_push(vm, Register::A, Register::F)],
        0xF6 => mk_inst![vm> "ORd8",    i_ord8(vm)],
        0xF7 => mk_inst![vm> "RST30h",  i_rst(vm, 0x30)],
        0xF8 => mk_inst![vm> "LDHLSPr8",  i_ldhlspr8(vm)],
        0xF9 => mk_inst![vm> "LDSPHL",  i_ldsphl(vm)],
        0xFA => mk_inst![vm> "LDAa16m", i_ldaa16m(vm)],
        0xFB => mk_inst![vm> "EI",      i_ei(vm)],
        0xFC => mk_inst![vm> "0xFC",    i_invalid(vm, 0xFC)],
        0xFD => mk_inst![vm> "0xFD",    i_invalid(vm, 0xFD)],
        0xFE => mk_inst![vm> "CPd8",    i_cpd8(vm)],
        0xFF => mk_inst![vm> "RST38h",  i_rst(vm, 0x38)],

        _ => panic!(format!("Missing instruction 0x{:02X} !", opcode)),
    }
}

/// Associate to each opcode:u8 it's instruction:Instruction in the 0xCB table
pub fn dispatch_cb(opcode : u8) -> Instruction {
    match opcode {
        0x00 => mk_inst![vm> "RLCB",     i_rlc(vm, Register::B)],
        0x01 => mk_inst![vm> "RLCC",     i_rlc(vm, Register::C)],
        0x02 => mk_inst![vm> "RLCD",     i_rlc(vm, Register::D)],
        0x03 => mk_inst![vm> "RLCE",     i_rlc(vm, Register::E)],
        0x04 => mk_inst![vm> "RLCH",     i_rlc(vm, Register::H)],
        0x05 => mk_inst![vm> "RLCL",     i_rlc(vm, Register::L)],
        0x06 => mk_inst![vm> "RLCHLm",   i_rlchlm(vm)],
        0x07 => mk_inst![vm> "RLCA",     i_rlc(vm, Register::A)],
        0x08 => mk_inst![vm> "RRCB",     i_rrc(vm, Register::B)],
        0x09 => mk_inst![vm> "RRCC",     i_rrc(vm, Register::C)],
        0x0A => mk_inst![vm> "RRCD",     i_rrc(vm, Register::D)],
        0x0B => mk_inst![vm> "RRCE",     i_rrc(vm, Register::E)],
        0x0C => mk_inst![vm> "RRCH",     i_rrc(vm, Register::H)],
        0x0D => mk_inst![vm> "RRCL",     i_rrc(vm, Register::L)],
        0x0E => mk_inst![vm> "RRCHLm",   i_rrchlm(vm)],
        0x0F => mk_inst![vm> "RRCA",     i_rrc(vm, Register::A)],

        0x10 => mk_inst![vm> "RLB",     i_rl(vm, Register::B)],
        0x11 => mk_inst![vm> "RLC",     i_rl(vm, Register::C)],
        0x12 => mk_inst![vm> "RLD",     i_rl(vm, Register::D)],
        0x13 => mk_inst![vm> "RLE",     i_rl(vm, Register::E)],
        0x14 => mk_inst![vm> "RLH",     i_rl(vm, Register::H)],
        0x15 => mk_inst![vm> "RLL",     i_rl(vm, Register::L)],
        0x16 => mk_inst![vm> "RLHLm",   i_rlhlm(vm)],
        0x17 => mk_inst![vm> "RLA",     i_rl(vm, Register::A)],
        0x18 => mk_inst![vm> "RRB",     i_rr(vm, Register::B)],
        0x19 => mk_inst![vm> "RRC",     i_rr(vm, Register::C)],
        0x1A => mk_inst![vm> "RRD",     i_rr(vm, Register::D)],
        0x1B => mk_inst![vm> "RRE",     i_rr(vm, Register::E)],
        0x1C => mk_inst![vm> "RRH",     i_rr(vm, Register::H)],
        0x1D => mk_inst![vm> "RRL",     i_rr(vm, Register::L)],
        0x1E => mk_inst![vm> "RRHLm",   i_rrhlm(vm)],
        0x1F => mk_inst![vm> "RRA",     i_rr(vm, Register::A)],

        0x20 => mk_inst![vm> "SLAB",     i_sla(vm, Register::B)],
        0x21 => mk_inst![vm> "SLAC",     i_sla(vm, Register::C)],
        0x22 => mk_inst![vm> "SLAD",     i_sla(vm, Register::D)],
        0x23 => mk_inst![vm> "SLAE",     i_sla(vm, Register::E)],
        0x24 => mk_inst![vm> "SLAH",     i_sla(vm, Register::H)],
        0x25 => mk_inst![vm> "SLAL",     i_sla(vm, Register::L)],
        0x26 => mk_inst![vm> "SLAHLm",   i_slahlm(vm)],
        0x27 => mk_inst![vm> "SLAA",     i_sla(vm, Register::A)],
        0x28 => mk_inst![vm> "SRAB",     i_sra(vm, Register::B)],
        0x29 => mk_inst![vm> "SRAC",     i_sra(vm, Register::C)],
        0x2A => mk_inst![vm> "SRAD",     i_sra(vm, Register::D)],
        0x2B => mk_inst![vm> "SRAE",     i_sra(vm, Register::E)],
        0x2C => mk_inst![vm> "SRAH",     i_sra(vm, Register::H)],
        0x2D => mk_inst![vm> "SRAL",     i_sra(vm, Register::L)],
        0x2E => mk_inst![vm> "SRAHLm",   i_srahlm(vm)],
        0x2F => mk_inst![vm> "SRAA",     i_sra(vm, Register::A)],

        0x30 => mk_inst![vm> "SWAPB",    i_swap(vm, Register::B)],
        0x31 => mk_inst![vm> "SWAPC",    i_swap(vm, Register::C)],
        0x32 => mk_inst![vm> "SWAPD",    i_swap(vm, Register::D)],
        0x33 => mk_inst![vm> "SWAPE",    i_swap(vm, Register::E)],
        0x34 => mk_inst![vm> "SWAPH",    i_swap(vm, Register::H)],
        0x35 => mk_inst![vm> "SWAPL",    i_swap(vm, Register::L)],
        0x36 => mk_inst![vm> "SWAPHLm",  i_swaphlm(vm)],
        0x37 => mk_inst![vm> "SWAPA",    i_swap(vm, Register::A)],
        0x38 => mk_inst![vm> "SRLB",     i_srl(vm, Register::B)],
        0x39 => mk_inst![vm> "SRLC",     i_srl(vm, Register::C)],
        0x3A => mk_inst![vm> "SRLD",     i_srl(vm, Register::D)],
        0x3B => mk_inst![vm> "SRLE",     i_srl(vm, Register::E)],
        0x3C => mk_inst![vm> "SRLH",     i_srl(vm, Register::H)],
        0x3D => mk_inst![vm> "SRLL",     i_srl(vm, Register::L)],
        0x3E => mk_inst![vm> "SRLHLm",   i_srlhlm(vm)],
        0x3F => mk_inst![vm> "SRLA",     i_srl(vm, Register::A)],

        0x40 => mk_inst![vm> "BIT0B",    i_bitr(vm, 0, Register::B)],
        0x41 => mk_inst![vm> "BIT0C",    i_bitr(vm, 0, Register::C)],
        0x42 => mk_inst![vm> "BIT0D",    i_bitr(vm, 0, Register::D)],
        0x43 => mk_inst![vm> "BIT0E",    i_bitr(vm, 0, Register::E)],
        0x44 => mk_inst![vm> "BIT0H",    i_bitr(vm, 0, Register::H)],
        0x45 => mk_inst![vm> "BIT0L",    i_bitr(vm, 0, Register::L)],
        0x46 => mk_inst![vm> "BIT0HLm",  i_bithlm(vm, 0)],
        0x47 => mk_inst![vm> "BIT0A",    i_bitr(vm, 0, Register::A)],
        0x48 => mk_inst![vm> "BIT1B",    i_bitr(vm, 1, Register::B)],
        0x49 => mk_inst![vm> "BIT1C",    i_bitr(vm, 1, Register::C)],
        0x4A => mk_inst![vm> "BIT1D",    i_bitr(vm, 1, Register::D)],
        0x4B => mk_inst![vm> "BIT1E",    i_bitr(vm, 1, Register::E)],
        0x4C => mk_inst![vm> "BIT1H",    i_bitr(vm, 1, Register::H)],
        0x4D => mk_inst![vm> "BIT1L",    i_bitr(vm, 1, Register::L)],
        0x4E => mk_inst![vm> "BIT1HLm",  i_bithlm(vm, 1)],
        0x4F => mk_inst![vm> "BIT1A",    i_bitr(vm, 1, Register::A)],

        0x50 => mk_inst![vm> "BIT2B",    i_bitr(vm, 2, Register::B)],
        0x51 => mk_inst![vm> "BIT2C",    i_bitr(vm, 2, Register::C)],
        0x52 => mk_inst![vm> "BIT2D",    i_bitr(vm, 2, Register::D)],
        0x53 => mk_inst![vm> "BIT2E",    i_bitr(vm, 2, Register::E)],
        0x54 => mk_inst![vm> "BIT2H",    i_bitr(vm, 2, Register::H)],
        0x55 => mk_inst![vm> "BIT2L",    i_bitr(vm, 2, Register::L)],
        0x56 => mk_inst![vm> "BIT2HLm",  i_bithlm(vm, 2)],
        0x57 => mk_inst![vm> "BIT2A",    i_bitr(vm, 2, Register::A)],
        0x58 => mk_inst![vm> "BIT3B",    i_bitr(vm, 3, Register::B)],
        0x59 => mk_inst![vm> "BIT3C",    i_bitr(vm, 3, Register::C)],
        0x5A => mk_inst![vm> "BIT3D",    i_bitr(vm, 3, Register::D)],
        0x5B => mk_inst![vm> "BIT3E",    i_bitr(vm, 3, Register::E)],
        0x5C => mk_inst![vm> "BIT3H",    i_bitr(vm, 3, Register::H)],
        0x5D => mk_inst![vm> "BIT3L",    i_bitr(vm, 3, Register::L)],
        0x5E => mk_inst![vm> "BIT3HLm",  i_bithlm(vm, 3)],
        0x5F => mk_inst![vm> "BIT3A",    i_bitr(vm, 3, Register::A)],

        0x60 => mk_inst![vm> "BIT4B",    i_bitr(vm, 4, Register::B)],
        0x61 => mk_inst![vm> "BIT4C",    i_bitr(vm, 4, Register::C)],
        0x62 => mk_inst![vm> "BIT4D",    i_bitr(vm, 4, Register::D)],
        0x63 => mk_inst![vm> "BIT4E",    i_bitr(vm, 4, Register::E)],
        0x64 => mk_inst![vm> "BIT4H",    i_bitr(vm, 4, Register::H)],
        0x65 => mk_inst![vm> "BIT4L",    i_bitr(vm, 4, Register::L)],
        0x66 => mk_inst![vm> "BIT4HLm",  i_bithlm(vm, 4)],
        0x67 => mk_inst![vm> "BIT4A",    i_bitr(vm, 4, Register::A)],
        0x68 => mk_inst![vm> "BIT5B",    i_bitr(vm, 5, Register::B)],
        0x69 => mk_inst![vm> "BIT5C",    i_bitr(vm, 5, Register::C)],
        0x6A => mk_inst![vm> "BIT5D",    i_bitr(vm, 5, Register::D)],
        0x6B => mk_inst![vm> "BIT5E",    i_bitr(vm, 5, Register::E)],
        0x6C => mk_inst![vm> "BIT5H",    i_bitr(vm, 5, Register::H)],
        0x6D => mk_inst![vm> "BIT5L",    i_bitr(vm, 5, Register::L)],
        0x6E => mk_inst![vm> "BIT5HLm",  i_bithlm(vm, 5)],
        0x6F => mk_inst![vm> "BIT5A",    i_bitr(vm, 5, Register::A)],

        0x70 => mk_inst![vm> "BIT6B",    i_bitr(vm, 6, Register::B)],
        0x71 => mk_inst![vm> "BIT6C",    i_bitr(vm, 6, Register::C)],
        0x72 => mk_inst![vm> "BIT6D",    i_bitr(vm, 6, Register::D)],
        0x73 => mk_inst![vm> "BIT6E",    i_bitr(vm, 6, Register::E)],
        0x74 => mk_inst![vm> "BIT6H",    i_bitr(vm, 6, Register::H)],
        0x75 => mk_inst![vm> "BIT6L",    i_bitr(vm, 6, Register::L)],
        0x76 => mk_inst![vm> "BIT6HLm",  i_bithlm(vm, 6)],
        0x77 => mk_inst![vm> "BIT6A",    i_bitr(vm, 6, Register::A)],
        0x78 => mk_inst![vm> "BIT7B",    i_bitr(vm, 7, Register::B)],
        0x79 => mk_inst![vm> "BIT7C",    i_bitr(vm, 7, Register::C)],
        0x7A => mk_inst![vm> "BIT7D",    i_bitr(vm, 7, Register::D)],
        0x7B => mk_inst![vm> "BIT7E",    i_bitr(vm, 7, Register::E)],
        0x7C => mk_inst![vm> "BIT7H",    i_bitr(vm, 7, Register::H)],
        0x7D => mk_inst![vm> "BIT7L",    i_bitr(vm, 7, Register::L)],
        0x7E => mk_inst![vm> "BIT7HLm",  i_bithlm(vm, 7)],
        0x7F => mk_inst![vm> "BIT7A",    i_bitr(vm, 7, Register::A)],

        0x80 => mk_inst![vm> "RES0B",    i_res(vm, 0, Register::B)],
        0x81 => mk_inst![vm> "RES0C",    i_res(vm, 0, Register::C)],
        0x82 => mk_inst![vm> "RES0D",    i_res(vm, 0, Register::D)],
        0x83 => mk_inst![vm> "RES0E",    i_res(vm, 0, Register::E)],
        0x84 => mk_inst![vm> "RES0H",    i_res(vm, 0, Register::H)],
        0x85 => mk_inst![vm> "RES0L",    i_res(vm, 0, Register::L)],
        0x86 => mk_inst![vm> "RES0HLm",  i_reshlm(vm, 0)],
        0x87 => mk_inst![vm> "RES0A",    i_res(vm, 0, Register::A)],
        0x88 => mk_inst![vm> "RES0B",    i_res(vm, 1, Register::B)],
        0x89 => mk_inst![vm> "RES0C",    i_res(vm, 1, Register::C)],
        0x8A => mk_inst![vm> "RES0D",    i_res(vm, 1, Register::D)],
        0x8B => mk_inst![vm> "RES0E",    i_res(vm, 1, Register::E)],
        0x8C => mk_inst![vm> "RES0H",    i_res(vm, 1, Register::H)],
        0x8D => mk_inst![vm> "RES0L",    i_res(vm, 1, Register::L)],
        0x8E => mk_inst![vm> "RES0HLm",  i_reshlm(vm, 1)],
        0x8F => mk_inst![vm> "RES0A",    i_res(vm, 1, Register::A)],

        0x90 => mk_inst![vm> "RES2B",    i_res(vm, 2, Register::B)],
        0x91 => mk_inst![vm> "RES2C",    i_res(vm, 2, Register::C)],
        0x92 => mk_inst![vm> "RES2D",    i_res(vm, 2, Register::D)],
        0x93 => mk_inst![vm> "RES2E",    i_res(vm, 2, Register::E)],
        0x94 => mk_inst![vm> "RES2H",    i_res(vm, 2, Register::H)],
        0x95 => mk_inst![vm> "RES2L",    i_res(vm, 2, Register::L)],
        0x96 => mk_inst![vm> "RES2HLm",  i_reshlm(vm, 2)],
        0x97 => mk_inst![vm> "RES2A",    i_res(vm, 2, Register::A)],
        0x98 => mk_inst![vm> "RES3B",    i_res(vm, 3, Register::B)],
        0x99 => mk_inst![vm> "RES3C",    i_res(vm, 3, Register::C)],
        0x9A => mk_inst![vm> "RES3D",    i_res(vm, 3, Register::D)],
        0x9B => mk_inst![vm> "RES3E",    i_res(vm, 3, Register::E)],
        0x9C => mk_inst![vm> "RES3H",    i_res(vm, 3, Register::H)],
        0x9D => mk_inst![vm> "RES3L",    i_res(vm, 3, Register::L)],
        0x9E => mk_inst![vm> "RES3HLm",  i_reshlm(vm, 3)],
        0x9F => mk_inst![vm> "RES3A",    i_res(vm, 3, Register::A)],

        0xA0 => mk_inst![vm> "RES4B",    i_res(vm, 4, Register::B)],
        0xA1 => mk_inst![vm> "RES4C",    i_res(vm, 4, Register::C)],
        0xA2 => mk_inst![vm> "RES4D",    i_res(vm, 4, Register::D)],
        0xA3 => mk_inst![vm> "RES4E",    i_res(vm, 4, Register::E)],
        0xA4 => mk_inst![vm> "RES4H",    i_res(vm, 4, Register::H)],
        0xA5 => mk_inst![vm> "RES4L",    i_res(vm, 4, Register::L)],
        0xA6 => mk_inst![vm> "RES4HLm",  i_reshlm(vm, 4)],
        0xA7 => mk_inst![vm> "RES4A",    i_res(vm, 4, Register::A)],
        0xA8 => mk_inst![vm> "RES5B",    i_res(vm, 5, Register::B)],
        0xA9 => mk_inst![vm> "RES5C",    i_res(vm, 5, Register::C)],
        0xAA => mk_inst![vm> "RES5D",    i_res(vm, 5, Register::D)],
        0xAB => mk_inst![vm> "RES5E",    i_res(vm, 5, Register::E)],
        0xAC => mk_inst![vm> "RES5H",    i_res(vm, 5, Register::H)],
        0xAD => mk_inst![vm> "RES5L",    i_res(vm, 5, Register::L)],
        0xAE => mk_inst![vm> "RES5HLm",  i_reshlm(vm, 5)],
        0xAF => mk_inst![vm> "RES5A",    i_res(vm, 5, Register::A)],

        0xB0 => mk_inst![vm> "RES6B",    i_res(vm, 6, Register::B)],
        0xB1 => mk_inst![vm> "RES6C",    i_res(vm, 6, Register::C)],
        0xB2 => mk_inst![vm> "RES6D",    i_res(vm, 6, Register::D)],
        0xB3 => mk_inst![vm> "RES6E",    i_res(vm, 6, Register::E)],
        0xB4 => mk_inst![vm> "RES6H",    i_res(vm, 6, Register::H)],
        0xB5 => mk_inst![vm> "RES6L",    i_res(vm, 6, Register::L)],
        0xB6 => mk_inst![vm> "RES6HLm",  i_reshlm(vm, 6)],
        0xB7 => mk_inst![vm> "RES6A",    i_res(vm, 6, Register::A)],
        0xB8 => mk_inst![vm> "RES7B",    i_res(vm, 7, Register::B)],
        0xB9 => mk_inst![vm> "RES7C",    i_res(vm, 7, Register::C)],
        0xBA => mk_inst![vm> "RES7D",    i_res(vm, 7, Register::D)],
        0xBB => mk_inst![vm> "RES7E",    i_res(vm, 7, Register::E)],
        0xBC => mk_inst![vm> "RES7H",    i_res(vm, 7, Register::H)],
        0xBD => mk_inst![vm> "RES7L",    i_res(vm, 7, Register::L)],
        0xBE => mk_inst![vm> "RES7HLm",  i_reshlm(vm, 7)],
        0xBF => mk_inst![vm> "RES7A",    i_res(vm, 7, Register::A)],

        0xC0 => mk_inst![vm> "SET0B",    i_set(vm, 0, Register::B)],
        0xC1 => mk_inst![vm> "SET0C",    i_set(vm, 0, Register::C)],
        0xC2 => mk_inst![vm> "SET0D",    i_set(vm, 0, Register::D)],
        0xC3 => mk_inst![vm> "SET0E",    i_set(vm, 0, Register::E)],
        0xC4 => mk_inst![vm> "SET0H",    i_set(vm, 0, Register::H)],
        0xC5 => mk_inst![vm> "SET0L",    i_set(vm, 0, Register::L)],
        0xC6 => mk_inst![vm> "SET0HLm",  i_sethlm(vm, 0)],
        0xC7 => mk_inst![vm> "SET0A",    i_set(vm, 0, Register::A)],
        0xC8 => mk_inst![vm> "SET0B",    i_set(vm, 1, Register::B)],
        0xC9 => mk_inst![vm> "SET0C",    i_set(vm, 1, Register::C)],
        0xCA => mk_inst![vm> "SET0D",    i_set(vm, 1, Register::D)],
        0xCB => mk_inst![vm> "SET0E",    i_set(vm, 1, Register::E)],
        0xCC => mk_inst![vm> "SET0H",    i_set(vm, 1, Register::H)],
        0xCD => mk_inst![vm> "SET0L",    i_set(vm, 1, Register::L)],
        0xCE => mk_inst![vm> "SET0HLm",  i_sethlm(vm, 1)],
        0xCF => mk_inst![vm> "SET0A",    i_set(vm, 1, Register::A)],

        0xD0 => mk_inst![vm> "SET2B",    i_set(vm, 2, Register::B)],
        0xD1 => mk_inst![vm> "SET2C",    i_set(vm, 2, Register::C)],
        0xD2 => mk_inst![vm> "SET2D",    i_set(vm, 2, Register::D)],
        0xD3 => mk_inst![vm> "SET2E",    i_set(vm, 2, Register::E)],
        0xD4 => mk_inst![vm> "SET2H",    i_set(vm, 2, Register::H)],
        0xD5 => mk_inst![vm> "SET2L",    i_set(vm, 2, Register::L)],
        0xD6 => mk_inst![vm> "SET2HLm",  i_sethlm(vm, 2)],
        0xD7 => mk_inst![vm> "SET2A",    i_set(vm, 2, Register::A)],
        0xD8 => mk_inst![vm> "SET3B",    i_set(vm, 3, Register::B)],
        0xD9 => mk_inst![vm> "SET3C",    i_set(vm, 3, Register::C)],
        0xDA => mk_inst![vm> "SET3D",    i_set(vm, 3, Register::D)],
        0xDB => mk_inst![vm> "SET3E",    i_set(vm, 3, Register::E)],
        0xDC => mk_inst![vm> "SET3H",    i_set(vm, 3, Register::H)],
        0xDD => mk_inst![vm> "SET3L",    i_set(vm, 3, Register::L)],
        0xDE => mk_inst![vm> "SET3HLm",  i_sethlm(vm, 3)],
        0xDF => mk_inst![vm> "SET3A",    i_set(vm, 3, Register::A)],

        0xE0 => mk_inst![vm> "SET4B",    i_set(vm, 4, Register::B)],
        0xE1 => mk_inst![vm> "SET4C",    i_set(vm, 4, Register::C)],
        0xE2 => mk_inst![vm> "SET4D",    i_set(vm, 4, Register::D)],
        0xE3 => mk_inst![vm> "SET4E",    i_set(vm, 4, Register::E)],
        0xE4 => mk_inst![vm> "SET4H",    i_set(vm, 4, Register::H)],
        0xE5 => mk_inst![vm> "SET4L",    i_set(vm, 4, Register::L)],
        0xE6 => mk_inst![vm> "SET4HLm",  i_sethlm(vm, 4)],
        0xE7 => mk_inst![vm> "SET4A",    i_set(vm, 4, Register::A)],
        0xE8 => mk_inst![vm> "SET5B",    i_set(vm, 5, Register::B)],
        0xE9 => mk_inst![vm> "SET5C",    i_set(vm, 5, Register::C)],
        0xEA => mk_inst![vm> "SET5D",    i_set(vm, 5, Register::D)],
        0xEB => mk_inst![vm> "SET5E",    i_set(vm, 5, Register::E)],
        0xEC => mk_inst![vm> "SET5H",    i_set(vm, 5, Register::H)],
        0xED => mk_inst![vm> "SET5L",    i_set(vm, 5, Register::L)],
        0xEE => mk_inst![vm> "SET5HLm",  i_sethlm(vm, 5)],
        0xEF => mk_inst![vm> "SET5A",    i_set(vm, 5, Register::A)],

        0xF0 => mk_inst![vm> "SET6B",    i_set(vm, 6, Register::B)],
        0xF1 => mk_inst![vm> "SET6C",    i_set(vm, 6, Register::C)],
        0xF2 => mk_inst![vm> "SET6D",    i_set(vm, 6, Register::D)],
        0xF3 => mk_inst![vm> "SET6E",    i_set(vm, 6, Register::E)],
        0xF4 => mk_inst![vm> "SET6H",    i_set(vm, 6, Register::H)],
        0xF5 => mk_inst![vm> "SET6L",    i_set(vm, 6, Register::L)],
        0xF6 => mk_inst![vm> "SET6HLm",  i_sethlm(vm, 6)],
        0xF7 => mk_inst![vm> "SET6A",    i_set(vm, 6, Register::A)],
        0xF8 => mk_inst![vm> "SET7B",    i_set(vm, 7, Register::B)],
        0xF9 => mk_inst![vm> "SET7C",    i_set(vm, 7, Register::C)],
        0xFA => mk_inst![vm> "SET7D",    i_set(vm, 7, Register::D)],
        0xFB => mk_inst![vm> "SET7E",    i_set(vm, 7, Register::E)],
        0xFC => mk_inst![vm> "SET7H",    i_set(vm, 7, Register::H)],
        0xFD => mk_inst![vm> "SET7L",    i_set(vm, 7, Register::L)],
        0xFE => mk_inst![vm> "SET7HLm",  i_sethlm(vm, 7)],
        0xFF => mk_inst![vm> "SET7A",    i_set(vm, 7, Register::A)],

        _ => panic!(format!("Missing instruction 0xCB:0x{:02X} !", opcode)),
    }
}

/////////////////////////////////////////
//
// Implementation of the CPU instructions
//
/////////////////////////////////////////

/// No Operation
pub fn i_nop(_ : &mut Vm) -> Clock {
    Clock { m:1, t:4 }
}

/// LD (Load) instruction
///
/// Syntax : `LD vm:Vm dst:Register src:Register`
///
/// > LD Register <- Register
pub fn i_ldrr(vm : &mut Vm, dst : Register, src : Register) -> Clock {
    reg![vm; dst] = reg![vm; src];
    Clock { m:1, t:4 }
}

/// Same as LD, but alow to use (h:l) on the right side
///
/// Syntax : `LDrr16m vm:Vm h:Register l:Register`
///
/// > LDrr16m Register <- (h:l)
pub fn i_ldrr16m(vm : &mut Vm, dst : Register, h : Register, l : Register) -> Clock {
    let addr = get_r16(vm, h, l);
    reg![vm ; dst] = mmu::rb(addr, vm);
    Clock { m:1, t:8 }
}

/// Same as LD, but alow to use (h:l) on the left side
///
/// Syntax : `LDr16mr vm:Vm h:Register l:Register`
///
/// > LDr16mr (h:l) <- Register
pub fn i_ldr16mr(vm : &mut Vm, h : Register, l : Register, src : Register) -> Clock {
    let addr = get_r16(vm, h, l);
    mmu::wb(addr, reg![vm ; src], vm);
    Clock { m:1, t:8 }
}

/// Store the value of A into (0xFF00 + C)
///
/// Syntax : `LDCmA vm:Vm`
///
/// > LDCmA (0xFF00 + C) <- A
pub fn i_ldcma(vm : &mut Vm) -> Clock {
    let addr = 0xFF00 + reg![vm ; Register::C] as u16;
    mmu::wb(addr, reg![vm ; Register::A], vm);
    Clock { m:1, t:8 }
}

/// Store the value of (0xFF00 + C) into A
///
/// Syntax : `LDACm vm:Vm`
///
/// > LDACm A <- (0xFF00 + C)
pub fn i_ldacm(vm : &mut Vm) -> Clock {
    let addr = 0xFF00 + reg![vm ; Register::C] as u16;
    reg![vm ; Register::A] = mmu::rb(addr, vm);
    Clock { m:1, t:8 }
}


/// Store the value of A into (0xFF00 + a8)
///
/// Syntax : `LDHa8mA vm:Vm`
///
/// > LDH (0xFF00 + a8) <- A
pub fn i_ldha8ma(vm : &mut Vm) -> Clock {
    let addr = 0xFF00 + read_program_byte(vm) as u16;
    mmu::wb(addr, reg![vm ; Register::A], vm);
    Clock { m:2, t:12 }
}

/// Store the value of (0xFF00 + a8) into A
///
/// Syntax : `LDHAa8m vm:Vm`
///
/// > LDH A <- (0xFF00 + a8)
pub fn i_ldhaa8m(vm : &mut Vm) -> Clock {
    let addr = 0xFF00 + read_program_byte(vm) as u16;
    reg![vm ; Register::A] = mmu::rb(addr, vm);
    Clock { m:2, t:12 }
}

/// Implementation for LD[I|D] (HL) A
pub fn i_ldmod_hlma(vm : &mut Vm, modificator : i16) -> Clock {
    mmu::wb(hl![vm], reg![vm ; Register::A], vm);

    let sum = hl![vm].wrapping_add(modificator as u16);
    set_hl!(vm, sum as u16);
    Clock { m:1, t:8 }
}

/// Implementation for LD[I|D] A (HL)
pub fn i_ldmod_ahlm(vm : &mut Vm, modificator : i16) -> Clock {
    reg![vm ; Register::A] = mmu::rb(hl![vm], vm);

    let sum = hl![vm].wrapping_add(modificator as u16);
    set_hl!(vm, sum);
    Clock { m:1, t:8 }
}

/// Load the value of A in (HL) and increment HL
///
/// > LDI (HL+) <- A
pub fn i_ldihlma(vm : &mut Vm) -> Clock {i_ldmod_hlma(vm, 1)}

/// Load the value of (HL) in A and increment HL
///
/// > LDI A <- (HL+)
pub fn i_ldiahlm(vm : &mut Vm) -> Clock {i_ldmod_ahlm(vm, 1)}

/// Load the value of A in (HL) and decrement HL
///
/// > LDD (HL-) <- A
pub fn i_lddhlma(vm : &mut Vm) -> Clock {i_ldmod_hlma(vm, -1)}

/// Load the value of (HL) in A and decrement HL
///
/// > LDD A <- (HL-)
pub fn i_lddahlm(vm : &mut Vm) -> Clock {i_ldmod_ahlm(vm, -1)}

/// LD Register <- immediate Word8
pub fn i_ldrd8(vm : &mut Vm, dst : Register) -> Clock {
    reg![vm ; dst] = read_program_byte(vm);
    Clock { m:2, t:8 }
}

/// LD (HL) <- immediate Word8
pub fn i_ldhlmd8(vm : &mut Vm) -> Clock {
    mmu::wb(hl![vm], read_program_byte(vm), vm);
    Clock { m:2, t:8 }
}

/// LD (a16) <- a where a16 means the next Word16 as an address
pub fn i_lda16ma(vm : &mut Vm) -> Clock {
    let a16 = read_program_word(vm);
    mmu::wb(a16, reg![vm ; Register::A], vm);
    Clock { m:3, t:12 }
}

/// LD a <- (a16) where a16 means the next Word16 as an address
pub fn i_ldaa16m(vm : &mut Vm) -> Clock {
    let a16 = read_program_word(vm);
    reg![vm ; Register::A] = mmu::rb(a16, vm);
    Clock { m:3, t:12 }
}

/// LD (a16) <- SP where a16 means the next Word16 as an address
pub fn i_lda16msp(vm : &mut Vm) -> Clock {
    let a16 = read_program_word(vm);
    mmu::ww(a16, sp![vm], vm);
    Clock { m:3, t:20 }
}

/// LD r16 <- d16 where d16 means direct Word8 value
pub fn i_ldr16d16(vm : &mut Vm, h : Register, l : Register) -> Clock {
    let d16 = read_program_word(vm);
    set_r16(vm, h, l, d16);
    Clock { m:3, t:12 }
}


/// LD SP <- d16 where d16 means direct Word8 value
pub fn i_ldspd16(vm : &mut Vm) -> Clock {
    let d16 = read_program_word(vm);
    sp![vm] = d16;
    Clock { m:3, t:12 }
}

/// LD SP <- HL
pub fn i_ldsphl(vm : &mut Vm) -> Clock {
    sp![vm] = hl![vm];
    Clock { m:1, t:8 }
}

/// Implement xoring the register A with the value src_val
pub fn i_xor_imp(src_val : u8, vm : &mut Vm) {
    reg![vm ; Register::A] ^= src_val;
    let result = reg![vm ; Register::A];
    set_flag(vm, Flag::Z, result == 0);
}

/// XOR the register A with a register src into A
/// Syntax : `XOR src:Register`
pub fn i_xorr(vm : &mut Vm, src : Register) -> Clock {
    reset_flags(vm);
    i_xor_imp(reg![vm ; src], vm);
    Clock { m:1, t:8 }
}

/// XOR the register A with (HL) into A
/// Syntax : `XORHLm`
pub fn i_xorhlm(vm : &mut Vm) -> Clock {
    reset_flags(vm);
    i_xor_imp(mmu::rb(hl![vm], vm), vm);
    Clock { m:1, t:8 }
}

/// XOR the register A with immediate word8 into A
/// Syntax : `XORd8`
pub fn i_xord8(vm : &mut Vm) -> Clock {
    reset_flags(vm);
    let d8 = read_program_byte(vm);
    i_xor_imp(d8, vm);
    Clock { m:1, t:8 }
}

/// Implement swap
pub fn i_swap_imp(value : u8, vm : &mut Vm) -> u8{
    let result = value << 4 | value >> 4;
    reset_flags(vm);
    set_flag(vm, Flag::Z, result == 0);
    return result;
}

/// Swap the bits 0-4 and 5-7 of the register `reg`
/// Syntax : `SWAP src:Register`
pub fn i_swap(vm : &mut Vm, src : Register) -> Clock {
    reg![vm ; src] = i_swap_imp(reg![vm ; src], vm);
    Clock { m:2, t:8 }
}

/// Swap the bits 0-4 and 5-7 of (HL)
/// Syntax : `SWAPHLm`
pub fn i_swaphlm(vm : &mut Vm) -> Clock {
    let result = i_swap_imp(mmu::rb(hl![vm], vm), vm);
    mmu::wb(hl![vm], result, vm);
    Clock { m:2, t:16 }
}

/// Implementation of OR of a value with the register A, stored into A
pub fn i_or_imp(src_val : u8, vm : &mut Vm) {
    reset_flags(vm);
    reg![vm ; Register::A] |= src_val;
    let result = reg![vm ; Register::A];
    reset_flags(vm);
    set_flag(vm, Flag::Z, result == 0);
}

/// Bitwise OR the register A with a register src into A
/// Syntax : `OR src`
pub fn i_orr(vm : &mut Vm, src : Register) -> Clock {
    i_or_imp(reg![vm ; src], vm);
    Clock { m:1, t:4 }
}

/// Bitwise OR the register A with (HL) into A
/// Syntax : `ORHLm`
pub fn i_orhlm(vm : &mut Vm) -> Clock {
    i_or_imp(mmu::rb(hl![vm], vm), vm);
    Clock { m:1, t:8 }
}

/// Bitwise OR the register A with the immediate word8 into A
/// Syntax : `ORd8`
pub fn i_ord8(vm : &mut Vm) -> Clock {
    let byte = read_program_byte(vm);
    i_or_imp(byte, vm);
    Clock { m:2, t:8 }
}

/// Implementation of AND of a value with the register A, stored into A
pub fn i_and_imp(src_val : u8, vm : &mut Vm) {
    reset_flags(vm);
    reg![vm ; Register::A] &= src_val;
    let result = reg![vm ; Register::A];
    reset_flags(vm);
    set_flag(vm, Flag::Z, result == 0);
    set_flag(vm, Flag::H, true);
}

/// Bitwise AND the register A with a register src into A
/// Syntax : `AND src`
pub fn i_andr(vm : &mut Vm, src : Register) -> Clock {
    i_and_imp(reg![vm ; src], vm);
    Clock { m:1, t:4 }
}

/// Bitwise AND the register A with (HL) into A
/// Syntax : `ANDHLm`
pub fn i_andhlm(vm : &mut Vm) -> Clock {
    i_and_imp(mmu::rb(hl![vm], vm), vm);
    Clock { m:1, t:8 }
}

/// Bitwise AND the register A with the immediate word8 into A
/// Syntax : `ANDd8`
pub fn i_andd8(vm : &mut Vm) -> Clock {
    let byte = read_program_byte(vm);
    i_and_imp(byte, vm);
    Clock { m:2, t:8 }
}

/// Implementation of the increment instruction (setting flags)
pub fn i_inc_impl(vm : &mut Vm, initial_val : u8, final_val : u8) {
    set_flag(vm, Flag::Z, final_val == 0);
    set_flag(vm, Flag::H, (initial_val & 0x0F) + 1 > 0x0F);
    set_flag(vm, Flag::N, false);
}

/// Increment the register given, and set Z, H as expected.
/// Always set N to 0.
///
/// Syntax : `INC reg:Register`
pub fn i_incr(vm : &mut Vm, reg : Register) -> Clock {
    let initial_val = reg![vm ; reg];
    reg![vm ; reg] = reg![vm ; reg].wrapping_add(1);
    let final_val = reg![vm ; reg];
    i_inc_impl(vm, initial_val, final_val);
    Clock { m:1, t:4 }
}

/// Increment (HL), and set Z, H as expected.
/// Always set N to 0.
///
/// Syntax : `INCHLm`
pub fn i_inchlm(vm : &mut Vm) -> Clock {
    let initial_val = mmu::rb(hl![vm], vm);
    let final_val = initial_val.wrapping_add(1);
    mmu::wb(hl![vm], final_val, vm);
    i_inc_impl(vm, initial_val, final_val);
    Clock { m:1, t:12 }
}

/// Increment the 16 bits register given.
/// Leave flags unaffected.
///
/// Syntax : `INC hight:Register low:Register`
pub fn i_incr16(vm : &mut Vm, h : Register, l : Register) -> Clock {
    let initial_val = get_r16(vm, h, l);
    let final_val = initial_val.wrapping_add(1);
    set_r16(vm, h, l, final_val);

    Clock { m:1, t:8 }
}

/// Increment the register SP
/// Leave flags unaffected.
///
/// Syntax : `INCSP`
pub fn i_incsp(vm : &mut Vm) -> Clock {
    sp![vm] = sp![vm].wrapping_add(1);

    Clock { m:1, t:8 }
}

/// Implementation of the increment instruction (setting flags)
pub fn i_dec_impl(vm : &mut Vm, initial_val : u8, final_val : u8) {
    set_flag(vm, Flag::Z, final_val == 0);
    set_flag(vm, Flag::H, initial_val & 0x0F == 0);
    set_flag(vm, Flag::N, true);
}

/// Decrement the register given, and set Z, H as expected.
/// Always set N to 0.
///
/// Syntax : `DEC reg:Register`
pub fn i_decr(vm : &mut Vm, reg : Register) -> Clock {
    let initial_val = reg![vm ; reg];
    let final_val = initial_val.wrapping_sub(1);
    reg![vm ; reg] = final_val;
    i_dec_impl(vm, initial_val, final_val);
    Clock { m:1, t:4 }
}

/// Decrement (HL), and set Z, H as expected.
/// Always set N to 0.
///
/// Syntax : `INCHLm`
pub fn i_dechlm(vm : &mut Vm) -> Clock {
    let initial_val = mmu::rb(hl![vm], vm);
    let final_val = initial_val.wrapping_sub(1);
    mmu::wb(hl![vm], final_val, vm);
    i_dec_impl(vm, initial_val, final_val);
    Clock { m:1, t:12 }
}

/// Decrement the 16 bits register given.
/// Leave flags unaffected.
///
/// Syntax : `DEC hight:Register low:Register`
pub fn i_decr16(vm : &mut Vm, h : Register, l : Register) -> Clock {
    let initial_val = get_r16(vm, h, l);
    let final_val = initial_val.wrapping_sub(1);
    set_r16(vm, h, l, final_val);

    Clock { m:1, t:8 }
}

/// Decrement the register SP
/// Leave flags unaffected.
///
/// Syntax : `DECSP`
pub fn i_decsp(vm : &mut Vm) -> Clock {
    sp![vm] = sp![vm].wrapping_sub(1);

    Clock { m:1, t:8 }
}

/// Compare src:Register with A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `CP src:Register`
pub fn i_cpr(vm : &mut Vm, src : Register) -> Clock {
    let input = reg![vm ; src];

    // Update flags and discard result
    i_sub_imp(vm, input);

    Clock { m:1, t:4 }
}

/// Compare (HL) with A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `CPHLm`
pub fn i_cphlm(vm : &mut Vm) -> Clock {
    let input = mmu::rb(hl![vm], vm);

    // Update flags and discard result
    i_sub_imp(vm, input);

    Clock { m:1, t:8 }
}

/// Compare direct Word8 with A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `CPd8`
pub fn i_cpd8(vm : &mut Vm) -> Clock {
    let input = read_program_byte(vm);

    // Update flags and discard result
    i_sub_imp(vm, input);

    Clock { m:2, t:8 }
}

/// Implement substracting value:u8 to the register A and set the flags
pub fn i_sub_imp(vm : &mut Vm, value : u8) -> u8 {
    let a = reg![vm ; Register::A];
    let b = value;
    let diff = a.wrapping_sub(b);
    reset_flags(vm);
    set_flag(vm, Flag::Z, diff == 0);
    set_flag(vm, Flag::N, true);
    set_flag(vm, Flag::H, 0x0F & b > 0x0F & a);
    set_flag(vm, Flag::C, b > a);
    return diff
}

/// Substract src:Register to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `SUB src:Register`
pub fn i_subr(vm : &mut Vm, src : Register) -> Clock {
    let input = reg![vm ; src];

    reg![vm ; Register::A] = i_sub_imp(vm, input);

    Clock { m:1, t:4 }
}

/// Substract (HL) to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `SUBHLm`
pub fn i_subhlm(vm : &mut Vm) -> Clock {
    let input = mmu::rb(hl![vm], vm);

    reg![vm ; Register::A] = i_sub_imp(vm, input);

    Clock { m:1, t:8 }
}

/// Substract direct Word8 to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `SUBd8`
pub fn i_subd8(vm : &mut Vm) -> Clock {
    let input = read_program_byte(vm);

    reg![vm ; Register::A] = i_sub_imp(vm, input);

    Clock { m:2, t:8 }
}

/// Implement substracting value:u8 and carry to the register A and set the flags
pub fn i_sbc_imp(vm : &mut Vm, value : u8) -> u8 {
    let carry = flag![vm ; Flag::C] as u8;
    let a = reg![vm ; Register::A];
    let b = value;
    let diff = a.wrapping_sub(b).wrapping_sub(carry);
    reset_flags(vm);
    set_flag(vm, Flag::Z, diff == 0);
    set_flag(vm, Flag::N, true);
    set_flag(vm, Flag::H, (0x0F & b) + carry > 0x0F & a);
    set_flag(vm, Flag::C, (carry as u16) + (b as u16) > a as u16);
    return diff
}

/// Substract src:Register + carry to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `SBC src:Register`
pub fn i_sbcr(vm : &mut Vm, src : Register) -> Clock {
    let input = reg![vm ; src];

    reg![vm ; Register::A] = i_sbc_imp(vm, input);

    Clock { m:1, t:4 }
}

/// Substract (HL) + carry to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `SBCHLm`
pub fn i_sbchlm(vm : &mut Vm) -> Clock {
    let input = mmu::rb(hl![vm], vm);

    reg![vm ; Register::A] = i_sbc_imp(vm, input);

    Clock { m:1, t:8 }
}

/// Substract direct Word8 + carry to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `SBCd8`
pub fn i_sbcd8(vm : &mut Vm) -> Clock {
    let input = read_program_byte(vm);

    reg![vm ; Register::A] = i_sbc_imp(vm, input);

    Clock { m:2, t:8 }
}

/// Implement adding value:u8 to the register A and set the flags
pub fn i_add_imp(vm : &mut Vm, value : u8) -> u8 {
    let a = reg![vm ; Register::A];
    let b = value;
    let sum = a.wrapping_add(b);
    reset_flags(vm);
    set_flag(vm, Flag::Z, sum == 0);
    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, (0x0F & a) + (0x0F & b) > 0xF);
    set_flag(vm, Flag::C, (b as u16) + (a as u16) > 0xFF);
    return sum
}

/// Add src:Register to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `ADD src:Register`
pub fn i_addr(vm : &mut Vm, src : Register) -> Clock {
    let input = reg![vm ; src];

    reg![vm ; Register::A] = i_add_imp(vm, input);

    Clock { m:1, t:4 }
}

/// Add (HL) to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `ADDHLm`
pub fn i_addhlm(vm : &mut Vm) -> Clock {
    let input = mmu::rb(hl![vm], vm);

    reg![vm ; Register::A] = i_add_imp(vm, input);

    Clock { m:1, t:8 }
}

/// Add direct Word8 to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `CPd8`
pub fn i_addd8(vm : &mut Vm) -> Clock {
    let input = read_program_byte(vm);

    reg![vm ; Register::A] = i_add_imp(vm, input);

    Clock { m:2, t:8 }
}

/// Implement 16bits ADD
///
/// Set Z H C.
pub fn i_add_imp16(vm : &mut Vm, a: u16, b : u16) -> u16 {
    let sum = a.wrapping_add(b);

    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, ((0x0FFF & a) + (0x0FFF & b)) & 0x1000 != 0);
    set_flag(vm, Flag::C, (b as u32) + (a as u32) > 0xFFFF);

    return sum
}

/// Add a r16 register to HL
///
/// Affect only flags H, N and C.
pub fn i_addhlr16(vm : &mut Vm, h : Register, l : Register) -> Clock {
    let a = hl![vm];
    let b = get_r16(vm, h, l);

    let sum = i_add_imp16(vm, a, b);
    set_hl!(vm, sum);

    Clock { m:1, t:8 }
}

/// Add SP to HL
///
/// Affect only flags H, N and C.
pub fn i_addhlsp(vm : &mut Vm) -> Clock {
    let a = hl![vm];
    let b = sp![vm];

    let sum = i_add_imp16(vm, a, b);
    set_hl!(vm, sum);

    Clock { m:1, t:8 }
}

/// Add direct Word8 to SP
///
/// Affect all flags.
pub fn i_addspr8(vm : &mut Vm) -> Clock {
    let a = sp![vm] as u16;
    let b = (read_program_byte(vm) as i8) as u16;

    let sum = a.wrapping_add(b as u16);

    reset_flags(vm);
    set_flag(vm, Flag::H, (0x0F & a) + (0x0F & b) > 0x0F);
    set_flag(vm, Flag::C, (a & 0xFF) + (b & 0xFF) > 0xFF);
    sp![vm] = sum;

    Clock { m:1, t:8 }
}

/// Load in HL the value of SP plus direct Word8
pub fn i_ldhlspr8(vm : &mut Vm) -> Clock {
    let a = sp![vm];
    let b = (read_program_byte(vm) as i8) as u16;

    let sum = a.wrapping_add(b as u16);

    reset_flags(vm);
    set_flag(vm, Flag::H, (0x0F & a) + (0x0F & b) > 0x0F);
    set_flag(vm, Flag::C, (a & 0xFF) + (b & 0xFF) > 0xFF);
    set_hl!(vm, sum);

    Clock { m:2, t: 12 }
}


/// Implement adding value:u8 + carry to the register A and set the flags
pub fn i_adc_imp(vm : &mut Vm, value : u8) -> u8 {
    let carry = flag![vm ; Flag::C] as u8;
    let a = reg![vm ; Register::A];
    let b = value;
    let sum = a.wrapping_add(b).wrapping_add(carry);
    reset_flags(vm);
    set_flag(vm, Flag::Z, sum == 0);
    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, (0x0F & a) + (0x0F & b) + carry > 0xF);
    set_flag(vm, Flag::C, (b as u16) + (a as u16) + (carry as u16) > 0xFF);
    return sum
}

/// Add src:Register + carry to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `ADC src:Register`
pub fn i_adcr(vm : &mut Vm, src : Register) -> Clock {
    let input = reg![vm ; src];

    reg![vm ; Register::A] = i_adc_imp(vm, input);

    Clock { m:1, t:4 }
}

/// Add (HL) +carry to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `ADCHLm`
pub fn i_adchlm(vm : &mut Vm) -> Clock {
    let input = mmu::rb(hl![vm], vm);

    reg![vm ; Register::A] = i_adc_imp(vm, input);

    Clock { m:1, t:8 }
}

/// Add direct Word8 + carry to A and set the flags Z/H/C.
/// Set register N to 1.
///
/// Syntax : `CPd8`
pub fn i_adcd8(vm : &mut Vm) -> Clock {
    let input = read_program_byte(vm);

    reg![vm ; Register::A] = i_adc_imp(vm, input);

    Clock { m:2, t:8 }
}

/// Test the bit bit from src.
///
/// Affect flags Z,N and H.
pub fn i_bitr(vm : &mut Vm, bit : usize, src : Register) -> Clock {
    let bit_value = reg![vm ; src] >> bit & 0x01;

    set_flag(vm, Flag::Z, bit_value == 0);
    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, true);

    Clock { m:2, t:8 }
}

/// Test the bit bit from (HL).
///
/// Affect flags Z,N and H.
pub fn i_bithlm(vm : &mut Vm, bit : usize) -> Clock {
    let value = mmu::rb(hl![vm], vm);
    let bit_value = value >> bit & 0x01;

    set_flag(vm, Flag::Z, bit_value == 0);
    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, true);

    Clock { m:2, t:16 }
}

/// Jump of the length given in direct Word8
///
/// Syntax : `JR`
pub fn i_jr(vm : &mut Vm) -> Clock {
    let byte = read_program_byte(vm);
    if byte <= 0x7F {
        pc![vm] = pc![vm].wrapping_add(byte as u16)
    }
    else {
        pc![vm] = pc![vm].wrapping_sub((0xFF - byte + 1) as u16)
    }
    Clock { m:2, t:12 }
}

/// Jump of the length given in direct Word8 if flag:Flag is set
///
/// Syntax : `JRf flag:Flag`
pub fn i_jrf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        i_jr(vm);
        Clock { m:2, t:12 }
    }
    else {
        read_program_byte(vm);
        Clock { m:2, t:8 }
    }
}

/// Jump of the length given in direct Word8 if flag:Flag is not set
///
/// Syntax : `JRnf flag:Flag`
pub fn i_jrnf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        read_program_byte(vm);
        Clock { m:2, t:8 }
    }
    else {
        i_jr(vm);
        Clock { m:2, t:12 }
    }
}

/// Read the next two bytes and jump to the address
///
/// Syntax : `JP`
pub fn i_jp(vm : &mut Vm) -> Clock {
    pc![vm] = read_program_word(vm);
    Clock { m:3, t:16 }
}

/// Read the next two bytes and jump to the address
///
/// Syntax : `JPHL`
pub fn i_jphl(vm : &mut Vm) -> Clock {
    pc![vm] = hl![vm];
    Clock { m:3, t:16 }
}

/// Jump of the address given in direct Word16 if flag:Flag is set
///
/// Syntax : `JPf flag:Flag`
pub fn i_jpf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        i_jp(vm);
        Clock { m:3, t:16 }
    }
    else {
        read_program_word(vm);
        Clock { m:3, t:12 }
    }
}

/// Jump of the address given in direct Word16 if flag:Flag is set
///
/// Syntax : `JPnf flag:Flag`
pub fn i_jpnf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        read_program_word(vm);
        Clock { m:3, t:12 }
    }
    else {
        i_jp(vm);
        Clock { m:3, t:16 }
    }
}

/// Push a r16 on the stack
///
/// Do note affect any register.
/// Syntax : `PUSH h:Register l:Register`
pub fn i_push(vm : &mut Vm, h : Register, l : Register) -> Clock {
    sp![vm] = sp![vm].wrapping_sub(2);
    mmu::ww(sp![vm], get_r16(vm, h, l), vm);
    Clock { m:1, t:16 }
}

/// Pop a r16 from the stack
///
/// Do note affect any register.
/// Syntax : `PUSH h:Register l:Register`
pub fn i_pop(vm : &mut Vm, h : Register, l : Register) -> Clock {
    let value = mmu::rw(sp![vm], vm);
    set_r16(vm, h, l, value);
    sp![vm] = sp![vm].wrapping_add(2);
    Clock { m:1, t:16 }
}

/// Call a function at addr a16
///
/// Actualy push PC on the stack and load a16 into PC
/// Syntax : `CALL`
pub fn i_call(vm : &mut Vm) -> Clock {
    let a16 = read_program_word(vm);

    // Push PC on the stack
    sp![vm] = sp![vm].wrapping_sub(2);
    mmu::ww(sp![vm], pc![vm], vm);

    // Update PC
    pc![vm] = a16;
    Clock { m:3, t:24 }
}

/// Call a function at addr a16 if flag is set
///
/// Actualy push PC on the stack and load a16 into PC
/// Syntax : `CALL flag:Flag`
pub fn i_callf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        i_call(vm);
        Clock { m:3, t:24 }
    }
    else {
        read_program_word(vm);
        Clock { m:3, t:12 }
    }
}

/// Call a function at addr a16 if flag is not set
///
/// Actualy push PC on the stack and load a16 into PC
/// Syntax : `CALL flag:Flag`
pub fn i_callnf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        read_program_word(vm);
        Clock { m:3, t:12 }
    }
    else {
        i_call(vm);
        Clock { m:3, t:24 }
    }
}

/// Return from a function
///
/// Actualy pop PC from the stack
/// Syntax : `RET`
pub fn i_ret(vm : &mut Vm) -> Clock {//TODO
    // Pop PC from the stack
    pc![vm] = mmu::rw(sp![vm], vm);
    sp![vm] = sp![vm].wrapping_add(2);

    Clock { m:1, t:16 }
}

/// Return from a function and enable interuptions
///
/// Actualy pop PC from the stack
/// Syntax : `RETI`
pub fn i_reti(vm : &mut Vm) -> Clock {
    vm.cpu.interrupt = InterruptState::IEnabled;
    i_ret(vm)
}

/// Return from a function if flag is set
///
/// Actualy pop PC from the stack
/// Syntax : `RET flag:Flag`
pub fn i_retf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        i_ret(vm);
        Clock { m:1, t:20 }
    }
    else {
        Clock { m:1, t:8 }
    }
}

/// Return from a function if flag is not set
///
/// Actualy pop PC from the stack
/// Syntax : `RET flag:Flag`
pub fn i_retnf(vm : &mut Vm, flag : Flag) -> Clock {
    if flag![vm ; flag] {
        Clock { m:1, t:8 }
    }
    else {
        i_ret(vm);
        Clock { m:1, t:20 }
    }
}

/// Implementation of RL
pub fn i_rl_imp(value : u8, vm : &mut Vm) -> u8 {
    let carry = flag![vm ; Flag::C] as u8;
    let result = (value << 1) | carry;

    reset_flags(vm);
    set_flag(vm, Flag::C, (value & 0x80) != 0); // Take value's bit 7
    set_flag(vm, Flag::Z, result == 0);

    return result;
}

/// Rotate Left through carry
///
/// Rotate the value in register reg 1 bit left.
/// Bit 7 goes in carry, and carry goes at reg's 0 bit.
///
/// Syntax : `RL reg:Register`
pub fn i_rl(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_rl_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}

/// Rotate Left through carry
///
/// Rotate the value in register `A` 1 bit left.
/// Bit 7 goes in carry, and carry goes at reg's 0 bit.
///
/// Syntax : `RLA`
pub fn i_rla(vm : &mut Vm) -> Clock {
    i_rl(vm, Register::A);
    set_flag(vm, Flag::Z, false);
    Clock { m:2, t:8 }
}

/// Rotate Left through carry
///
/// Rotate the value in (HL) 1 bit left.
/// Bit 7 goes in carry, and carry goes at (HL)'s 0 bit.
///
/// Syntax : `RLHLm`
pub fn i_rlhlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_rl_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Implementation of RR
pub fn i_rr_imp(value : u8, vm : &mut Vm) -> u8 {
    let carry = flag![vm ; Flag::C] as u8;
    let result = (value >> 1) | carry << 7;

    reset_flags(vm);
    set_flag(vm, Flag::C, (value & 0x01) != 0); // Take value's bit 0
    set_flag(vm, Flag::Z, result == 0);

    return result;
}

/// Rotate Right through carry
///
/// Rotate the value in register reg 1 bit right.
/// Bit 0 goes in carry, and carry goes at reg's 7 bit.
///
/// Syntax : `RR reg:Register`
pub fn i_rr(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_rr_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}


/// Rotate Right through carry
///
/// Rotate the value in register reg 1 bit right.
/// Bit 0 goes in carry, and carry goes at reg's 7 bit.
///
/// Reset Z flag.
///
/// Syntax : `RR reg:Register`
pub fn i_rra(vm : &mut Vm) -> Clock {
    i_rr(vm, Register::A);
    set_flag(vm, Flag::Z, false);
    Clock { m:1, t:4 }
}


/// Rotate Right through carry
///
/// Rotate the value in (HL) 1 bit right.
/// Bit 0 goes in carry, and carry goes at (HL)'s 7 bit.
///
/// Syntax : `RRHLm`
pub fn i_rrhlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_rr_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Implementation of SLA
pub fn i_sla_imp(value : u8, vm : &mut Vm) -> u8 {
    let result = value << 1;

    reset_flags(vm);
    set_flag(vm, Flag::C, value & 0x80 != 0); // Take value's bit 7
    set_flag(vm, Flag::Z, result == 0);

    return result;
}

/// Shift left
///
/// Shift the value in the register `reg` of 1 to the left.
/// Bit 7 goes in carry, and register's bit 0 is set to 0.
///
/// Syntax : `SLA reg:Register`
pub fn i_sla(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_sla_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}

/// Shift left
///
/// Shift the value in (HL) of 1 to the left.
/// Bit 7 goes in carry, and (HL)'s bit 0 is set to 0.
///
/// Syntax : `SLAHLm`
pub fn i_slahlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_sla_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Implementation of SRA
pub fn i_sra_imp(value : u8, vm : &mut Vm) -> u8 {
    let result = value >> 1 | value & 0x80;

    reset_flags(vm);
    set_flag(vm, Flag::C, value & 0x01 != 0); // Take value's bit 0
    set_flag(vm, Flag::Z, result == 0);

    return result;
}

/// Shift right
///
/// Shift the value in the register `reg` of 1 to the right.
/// Bit 7 stay inchanged, and register's bit 0 goes in carry.
///
/// Syntax : `SRA reg:Register`
pub fn i_sra(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_sra_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}

/// Shift right
///
/// Shift the value in (HL) of 1 to the right.
/// Bit 7 stay inchanged, and register's bit 0 goes in carry.
///
/// Syntax : `SRAHLm`
pub fn i_srahlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_sra_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Implementation of SRL
pub fn i_srl_imp(value : u8, vm : &mut Vm) -> u8 {
    let result = value >> 1;

    reset_flags(vm);
    set_flag(vm, Flag::C, value & 0x01 != 0); // Take value's bit 0
    set_flag(vm, Flag::Z, result == 0);

    return result;
}

/// Shift right
///
/// Shift the value in the register `reg` of 1 to the right.
/// Bit 7 is set to 0, and register's bit 0 goes in carry.
///
/// Syntax : `SRL reg:Register`
pub fn i_srl(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_srl_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}

/// Shift right
///
/// Shift the value in (HL) of 1 to the right.
/// Bit 7 is set to 0, and register's bit 0 goes in carry.
///
/// Syntax : `SRLHLm`
pub fn i_srlhlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_srl_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Implementation of RLC
pub fn i_rlc_imp(value : u8, vm : &mut Vm) -> u8 {
    let result = (value << 1) | (value >> 7);

    reset_flags(vm);
    // println!("rlca {:08b} {:08b}", value, result);
    set_flag(vm, Flag::C, (value >> 7) != 0);
    set_flag(vm, Flag::Z, result == 0);
    // println!("Z:{}, N:{}, H:{}, C:{}",
    //          flag![vm ; Flag::Z],
    //          flag![vm ; Flag::N],
    //          flag![vm ; Flag::H],
    //          flag![vm ; Flag::C]);

    return result;
}

/// Rotate Left
///
/// Rotate the value in register `reg` 1 bit on the left.
/// Bit 7 goes in carry.
///
/// Syntax : `RLC reg:Register`
pub fn i_rlc(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_rlc_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}

/// Rotate Left
///
/// Rotate the value in register `A` 1 bit on the left.
/// Bit 7 goes in carry.
///
/// Always reset Z flag
///
/// Syntax : `RLCA`
pub fn i_rlca(vm : &mut Vm) -> Clock {
    i_rlc(vm, Register::A);
    set_flag(vm, Flag::Z, false);
    Clock { m:1, t:4 }
}

/// Rotate Left
///
/// Rotate the value in (HL) 1 bit on the left.
/// Bit 7 goes in carry.
///
/// Syntax : `RLCHLm`
pub fn i_rlchlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_rlc_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Implementation of RRC
pub fn i_rrc_imp(value : u8, vm : &mut Vm) -> u8 {
    let result = (value >> 1) | (value << 7);

    reset_flags(vm);
    set_flag(vm, Flag::C, (value & 0x01) != 0); // Take value's bit 0
    set_flag(vm, Flag::Z, result == 0);

    return result;
}


/// Rotate Right
///
/// Rotate the value in register `reg` 1 bit on the right.
/// Bit 0 goes in carry.
///
/// Syntax : `RRC reg:Register`
pub fn i_rrc(vm : &mut Vm, reg : Register) -> Clock {
    reg![vm ; reg] = i_rrc_imp(reg![vm ; reg], vm);
    Clock { m:2, t:8 }
}

/// Rotate Right
///
/// Rotate the value in register `A` 1 bit on the right.
/// Bit 0 goes in carry.
///
/// Reset Z flag.
///
/// Syntax : `RRCA`
pub fn i_rrca(vm : &mut Vm) -> Clock {
    i_rrc(vm, Register::A);
    set_flag(vm, Flag::Z, false);
    Clock { m:1, t:4 }
}

/// Rotate Right
///
/// Rotate the value in (HL) 1 bit on the right.
/// Bit 0 goes in carry.
///
/// Syntax : `RRHLm`
pub fn i_rrchlm(vm : &mut Vm) -> Clock {
    // Read value
    let value = mmu::rb(hl![vm], vm);
    let result = i_rrc_imp(value, vm);
    // Write value
    mmu::wb(hl![vm], result, vm);

    Clock { m:2, t:16 }
}

/// Disable Interruptions
///
/// Syntax : `DI`
pub fn i_di(vm : &mut Vm) -> Clock {
    vm.cpu.interrupt = InterruptState::IDisableNextInst;
    Clock { m:1, t:4 }
}

/// Enable Interruptions
///
/// Syntax : `DI`
pub fn i_ei(vm : &mut Vm) -> Clock {
    vm.cpu.interrupt = InterruptState::IEnableNextInst;
    Clock { m:1, t:4 }
}

/// Binary complement to A register
///
/// A <- ~A
/// Syntax : `CPL`
pub fn i_cpl(vm : &mut Vm) -> Clock {
    reg![vm ; Register::A] = !reg![vm ; Register::A];

    set_flag(vm, Flag::N, true);
    set_flag(vm, Flag::H, true);

    Clock { m:1, t:4 }
}

/// Complement of carry flag
///
/// Carry <- ~Carry
/// Syntax : `CCF`
pub fn i_ccf(vm : &mut Vm) -> Clock {
    let carry = flag![vm ; Flag::C];

    set_flag(vm, Flag::C, !carry);
    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, false);

    Clock { m:1, t:4 }
}

/// RST : Push the stack and jump to a predetermined addr
///
/// Syntax : `RST addr:u16`
pub fn i_rst(vm : &mut Vm, addr : u16) -> Clock {
    // Push PC on the stack
    sp![vm] = sp![vm].wrapping_sub(2);
    mmu::ww(sp![vm], pc![vm], vm);

    // Update PC
    pc![vm] = addr;
    Clock { m:1, t:16 }
}

/// SCF : Set Carry Flag
///
/// Syntax : `SCF`
pub fn i_scf(vm : &mut Vm) -> Clock {
    set_flag(vm, Flag::N, false);
    set_flag(vm, Flag::H, false);
    set_flag(vm, Flag::C, true);
    Clock { m:1, t:4 }
}

/// Set the bit `bit` of the register `reg`
///
/// Syntax : `SET bit reg`
pub fn i_set(vm : &mut Vm, bit : u8, reg : Register) -> Clock {
    reg![vm ; reg] = reg![vm ; reg] | (1 << bit);
    Clock { m:2, t:8 }
}

/// Set the bit `bit` of (HL)
///
/// Syntax : `SET bit`
pub fn i_sethlm(vm : &mut Vm, bit : u8) -> Clock {
    let value = mmu::rb(hl![vm], vm);
    let result = value | (1 << bit);
    mmu::wb(hl![vm], result, vm);
    Clock { m:2, t:16 }
}


/// Reset the bit `bit` of the register `reg`
///
/// Syntax : `RES bit reg`
pub fn i_res(vm : &mut Vm, bit : u8, reg : Register) -> Clock {
    reg![vm ; reg] = reg![vm ; reg] & !(1 << bit);
    Clock { m:2, t:8 }
}

/// Reset the bit `bit` of (HL)
///
/// Syntax : `RES bit`
pub fn i_reshlm(vm : &mut Vm, bit : u8) -> Clock {
    let value = mmu::rb(hl![vm], vm);
    let result = value & !(1 << bit);
    mmu::wb(hl![vm], result, vm);
    Clock { m:2, t:16 }
}

/// Decimal Adjust Accumulator
///
/// This instruction adjust the accumulator
/// after an addition or substraction in the
/// case the numbers was represented in
/// packed BCD (Binary-coded decimal).
///
/// See http://www.z80.info/z80syntx.htm#DAA
/// and http://forums.nesdev.com/viewtopic.php?t=9088
///
/// Syntax : `DAA`
pub fn i_daa(vm : &mut Vm) -> Clock {
    let c = flag![vm ; Flag::C];
    let h = flag![vm ; Flag::H];

    let mut result = reg![vm ; Register::A] as u16;

    // In case of a substraction
    if flag![vm ; Flag::N] {
        if h {result = (result - 0x06) & 0xFF};
        if c {result -= 0x60};
    }
    // In case of an addition
    else {
        if h || (result & 0xF) > 9 {result += 0x06};
        if c || result > 0x9F      {result += 0x60};
    }

    reg![vm; Register::A] = result as u8;

    set_flag(vm, Flag::Z, result == 0);
    set_flag(vm, Flag::H, false);

    // Carry is unchanged unless there is a carry
    if result & 0x100 != 0 {
        set_flag(vm, Flag::C, true);
    }

    Clock { m:1, t:4 }
}

/// Invalid instruction
///
/// This opcode shouldn't be called.
///
/// The emulator just ignore it
pub fn i_invalid(vm : &mut Vm, opcode : u8) -> Clock {
    println!("Warning: Invalid opcode 0x{:02X}", opcode);
    Clock { m:1, t:4 }
}
