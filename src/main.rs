#![no_std]
#![no_main]

use core::cell::RefCell;
use cortex_m_rt::{entry, exception};
#[cfg(feature = "defmt")]
use defmt_rtt as _;
use embassy_boot_stm32::*;
use embassy_boot_stm32::{AlignedBuffer, FirmwareUpdaterConfig};
use embassy_stm32::flash::{Flash, BANK1_REGION3, WRITE_SIZE};
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::time::Hertz;
use embassy_stm32::usart::{BufferedUart, Config};
use embassy_stm32::{bind_interrupts, peripherals, usart};
use embassy_sync::blocking_mutex::Mutex;
use embedded_io::{Read, Write};
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
});

#[entry]
fn main() -> ! {
    let config = {
        use embassy_stm32::rcc::*;

        let mut config = embassy_stm32::Config::default();
        config.rcc.hse = Some(Hse {
            freq: Hertz::mhz(25),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll_src = PllSource::HSE;
        config.rcc.pll = Some(Pll {
            prediv: PllPreDiv::DIV25,
            mul: PllMul::MUL336,
            divp: Some(PllPDiv::DIV2),
            divq: Some(PllQDiv::DIV7),
            divr: None,
        });
        config.rcc.sys = Sysclk::PLL1_P;

        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV4;
        config.rcc.apb2_pre = APBPrescaler::DIV2;

        // reference your chip's manual for proper clock settings; this config
        // is recommended for a 32 bit frame at 48 kHz sample rate
        config.rcc.plli2s = Some(Pll {
            prediv: PllPreDiv::DIV25,
            mul: PllMul::MUL336,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV5),
        });
        config.enable_debug_during_sleep = true;

        config
    };
    let p = embassy_stm32::init(config);
    #[cfg(feature = "defmt")]
    defmt::info!("Hello World!");
    for _ in 0..10000 {
        cortex_m::asm::nop();
    }
    let layout = Flash::new_blocking(p.FLASH).into_blocking_regions();
    let flash_state = Mutex::new(RefCell::new(layout.bank1_region1));
    let flash_active_dfu = Mutex::new(RefCell::new(layout.bank1_region3));

    let config = BootLoaderConfig::from_linkerfile_blocking(&flash_active_dfu, &flash_active_dfu, &flash_state);
    let active_offset = config.active.offset();
    let mut led = Output::new(p.PC15, Level::High, Speed::Low);
    let button = Input::new(p.PE1, Pull::Up);
    let updater_config = FirmwareUpdaterConfig::from_linkerfile_blocking(&flash_active_dfu, &flash_state);
    let mut magic = AlignedBuffer([0; WRITE_SIZE]);
    let mut updater = BlockingFirmwareUpdater::new(updater_config, &mut magic.0);
    led.set_high();
    if button.is_low() {
        #[cfg(feature = "defmt")]
        defmt::info!("Enter DFU mode");
        match updater.mark_dfu() {
            Err(e) => {
                #[cfg(feature = "defmt")]
                defmt::info!("err {:?}", e);
            },
            Ok(_) => {}
        };
    }
    let bl = BootLoader::prepare::<_, _, _, 2048>(config);

    if bl.state == State::DfuDetach {
        led.set_low();
        let mut config = Config::default();
        config.baudrate = 115_200;
        static TX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
        let tx_buf = &mut TX_BUF.init([0; 128])[..];
        static RX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
        let rx_buf = &mut RX_BUF.init([0; 128])[..];
        let usart =
            BufferedUart::new(p.USART1, p.PA10, p.PA9, tx_buf, rx_buf, Irqs, config).unwrap();
        let (mut usr_tx, mut usr_rx) = usart.split();
        let mut fw_raw = [0u8; 65537]; // 1 (end flag), 16384 (payload)
        let mut offset = 0;
        loop {
            usr_tx.write_all("send ot".as_bytes()).unwrap();
            usr_rx.read_exact(&mut fw_raw).unwrap();
            #[cfg(feature = "defmt")]
            defmt::info!("Hello World! 1 {}", fw_raw[0]);
            updater.write_firmware(offset, &fw_raw[1..]).unwrap();
            updater.mark_updated().unwrap();
            #[cfg(feature = "defmt")]
            defmt::info!("Hello World! 2");
            offset += 65536;
            led.toggle();
            if fw_raw[0] != 0 {
                //last packet
                #[cfg(feature = "defmt")]
                defmt::info!("Hello World! 3");
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    }

    led.set_high();
    unsafe { bl.load(BANK1_REGION3.base + active_offset) }
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
