#![no_std]
#![no_main]

use core::cell::RefCell;
use cortex_m_rt::{entry, exception};
#[cfg(feature = "defmt")]
use defmt_rtt as _;
use embassy_boot_stm32::*;
use embassy_stm32::time::Hertz;
use embassy_stm32::flash::{Flash, BANK1_REGION};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_sync::blocking_mutex::Mutex;

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
    // Prevent a hard fault when accessing flash 'too early' after boot.
    //#[cfg(feature = "defmt")]
    for _ in 0..1000000 {
        cortex_m::asm::nop();
    }
    let layout = Flash::new_blocking(p.FLASH).into_blocking_regions();
    let flash = Mutex::new(RefCell::new(layout.bank1_region));

    let config = BootLoaderConfig::from_linkerfile_blocking(&flash, &flash,
                                                            &flash);
    let active_offset = config.active.offset();
    let mut _led = Output::new(p.PC13, Level::High, Speed::Low);
    let bl = BootLoader::prepare::<_, _, _, 2048>(config);

/*    if bl.state == State::DfuDetach {
        let mut usb_config = embassy_stm32::usb::Config::default();
        usb_config.vbus_detection = false;
        let mut ep_out_buffer = [0u8; 256];
        let driver = Driver::new_fs(p.USB_OTG_FS, Irqs, p.PA12, p.PA11,
                                    &mut ep_out_buffer, usb_config);
        let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
        config.manufacturer = Some("Embassy");
        config.product = Some("USB-DFU Bootloader example");
        config.serial_number = Some("1235678");

        let fw_config = FirmwareUpdaterConfig::from_linkerfile_blocking(&flash2,
                                                                        &flash);
        let mut buffer = AlignedBuffer([0; WRITE_SIZE]);
        let updater = BlockingFirmwareUpdater::new(fw_config, &mut buffer.0[..]);

        let mut config_descriptor = [0; 256];
        let mut bos_descriptor = [0; 256];
        let mut control_buf = [0; 2048];

        #[cfg(not(feature = "verify"))]
        let mut state = Control::new(updater, DfuAttributes::CAN_DOWNLOAD,
                                        ResetImmediate);

        #[cfg(feature = "verify")]
        let mut state = Control::new(updater, DfuAttributes::CAN_DOWNLOAD,
                                        ResetImmediate, PUBLIC_SIGNING_KEY);

        led.set_low();
        let mut builder = Builder::new(
            driver,
            config,
            &mut config_descriptor,
            &mut bos_descriptor,
            &mut [],
            &mut control_buf,
        );

        usb_dfu::<_, _, _, _, 2048>(&mut builder, &mut state, |_func| {});

        let mut dev = builder.build();
        embassy_futures::block_on(dev.run());
    }*/

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
