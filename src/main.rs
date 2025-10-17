#![no_std]
#![no_main]

use core::cell::RefCell;
use cortex_m_rt::{entry, exception};
#[cfg(feature = "defmt")]
use defmt_rtt as _;
use embassy_boot_stm32::*;
use embassy_boot_stm32::{AlignedBuffer, FirmwareUpdaterConfig};
use embassy_stm32::flash::{Flash, BANK1_REGION, WRITE_SIZE};
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::time::Hertz;
use embassy_stm32::usart::{BufferedUart, Config};
use embassy_stm32::{bind_interrupts, peripherals, usart};
use embassy_sync::blocking_mutex::Mutex;
use embedded_io::{Read, Write};
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USART2 => usart::BufferedInterruptHandler<peripherals::USART2>;
});

#[entry]
fn main() -> ! {
    let mut config = embassy_stm32::Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.hse = Some(Hse {
            freq: Hertz::mhz(8),
            mode: HseMode::Bypass,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL9,
        });
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
    }
    let p = embassy_stm32::init(config);
    for _ in 0..1000000 {
        cortex_m::asm::nop();
    }
    let layout = Flash::new_blocking(p.FLASH).into_blocking_regions();
    let flash = Mutex::new(RefCell::new(layout.bank1_region));

    let config = BootLoaderConfig::from_linkerfile_blocking(&flash, &flash, &flash);
    let active_offset = config.active.offset();
    let mut led = Output::new(p.PA12, Level::High, Speed::Low);
    let button = Input::new(p.PC5, Pull::None);
    let updater_config = FirmwareUpdaterConfig::from_linkerfile_blocking(&flash, &flash);
    let mut magic = AlignedBuffer([0; WRITE_SIZE]);
    let mut updater = BlockingFirmwareUpdater::new(updater_config, &mut magic.0);
    if button.is_low() {
        updater.mark_dfu().unwrap();
    }
    let bl = BootLoader::prepare::<_, _, _, 2048>(config);

    if bl.state == State::DfuDetach {
        let mut config = Config::default();
        config.baudrate = 2_000_000;
        static TX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
        let tx_buf = &mut TX_BUF.init([0; 128])[..];
        static RX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
        let rx_buf = &mut RX_BUF.init([0; 128])[..];
        let usart =
            BufferedUart::new(p.USART2, p.PA3, p.PA2, tx_buf, rx_buf, Irqs, config).unwrap();
        let (mut usr_tx, mut usr_rx) = usart.split();
        let mut fw_raw = [0u8; 2049]; // 1 (end flag), 2048 (payload)
        let mut offset = 0;
        loop {
            usr_tx.write_all("send ot".as_bytes()).unwrap();
            usr_rx.read_exact(&mut fw_raw).unwrap();
            updater.write_firmware(offset, &fw_raw[1..]).unwrap();
            offset += 2048;
            led.toggle();
            if fw_raw[0] != 0 {
                //last packet
                updater.mark_updated().unwrap();
                led.set_low();
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    }

    unsafe { bl.load(BANK1_REGION.base + active_offset) }
}

#[no_mangle]
#[cfg_attr(target_os = "none", link_section = ".HardFault.user")]
unsafe extern "C" fn HardFault() {
    cortex_m::peripheral::SCB::sys_reset();
}

#[exception]
unsafe fn DefaultHandler(_: i16) -> ! {
    const SCB_ICSR: *const u32 = 0xE000_ED04 as *const u32;
    let irqn = core::ptr::read_volatile(SCB_ICSR) as u8 as i16 - 16;

    panic!("DefaultHandler #{:?}", irqn);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    cortex_m::asm::udf();
}
