#![no_std]
#![no_main]

use core::cell::RefCell;

use cortex_m_rt::{entry, exception};
#[cfg(feature = "defmt")]
use defmt_rtt as _;
use embassy_boot_stm32::*;
use embassy_stm32::time::Hertz;
use embassy_stm32::flash::{Flash, BANK1_REGION, WRITE_SIZE};
//use embassy_stm32::rcc::WPAN_DEFAULT;
use embassy_stm32::usb::Driver;
use embassy_stm32::gpio::{AnyPin, Level, Output, Speed};
use embassy_stm32::{bind_interrupts, peripherals, usb};
use embassy_sync::blocking_mutex::Mutex;
use embassy_usb::{msos, Builder};
use embassy_usb_dfu::consts::DfuAttributes;
use embassy_usb_dfu::{usb_dfu, Control, ResetImmediate};

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});

// This is a randomly generated GUID to allow clients on Windows to find your device.
//
// N.B. update to a custom GUID for your own device!
const DEVICE_INTERFACE_GUIDS: &[&str] = &["{EAA9A5DC-30BA-44BC-9232-606CDC875321}"];

// This is a randomly generated example key.
//
// N.B. Please replace with your own!
#[cfg(feature = "verify")]
static PUBLIC_SIGNING_KEY: &[u8; 32] = include_bytes!("../secrets/key.pub.short");

#[entry]
fn main() -> ! {
    let mut config = embassy_stm32::Config::default();
    {
        use embassy_stm32::rcc::*;
        // 80Mhz clock (Source: 8 / SrcDiv: 1 * PllMul 20 / ClkDiv 2)
        // 80MHz highest frequency for flash 0 wait.
        config.rcc.sys = Sysclk::PLL1_R;
        config.rcc.hse = Some(Hse {
            freq: Hertz::mhz(8),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL48,
            divp: None,
            divq: Some(PllQDiv::DIV8),
            divr: Some(PllRDiv::DIV6), // sysclk 80Mhz clock (8 / 1 * 20 / 2)
        });
        config.rcc.mux.clk48sel = mux::Clk48sel::PLL1_Q;
    }
    let p = embassy_stm32::init(config);

    // Prevent a hard fault when accessing flash 'too early' after boot.
    //#[cfg(feature = "defmt")]
    for _ in 0..1000000 {
        cortex_m::asm::nop();
    }
    let layout = Flash::new_blocking(p.FLASH).into_blocking_regions();
    let flash = Mutex::new(RefCell::new(layout.bank1_region));

    let config = BootLoaderConfig::from_linkerfile_blocking(&flash, &flash, &flash);
    let active_offset = config.active.offset();
    let mut led = Output::new(p.PC13, Level::High, Speed::Low);
    led.set_low();
    let bl = BootLoader::prepare::<_, _, _, 2048>(config);

    if bl.state == State::DfuDetach {
        //let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
        let mut usb_config = embassy_stm32::usb::Config::default();
        usb_config.vbus_detection = false;
        let mut ep_out_buffer = [0u8; 256];
        let driver = Driver::new_fs(p.USB_OTG_FS, Irqs, p.PA12, p.PA11, &mut ep_out_buffer, usb_config);
        let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
        config.manufacturer = Some("Embassy");
        config.product = Some("USB-DFU Bootloader example");
        config.serial_number = Some("1235678");

        let fw_config = FirmwareUpdaterConfig::from_linkerfile_blocking(&flash, &flash);
        let mut buffer = AlignedBuffer([0; WRITE_SIZE]);
        let updater = BlockingFirmwareUpdater::new(fw_config, &mut buffer.0[..]);

        let mut config_descriptor = [0; 256];
        let mut bos_descriptor = [0; 256];
        let mut control_buf = [0; 4096];

        #[cfg(not(feature = "verify"))]
        let mut state = Control::new(updater, DfuAttributes::CAN_DOWNLOAD, ResetImmediate);

        #[cfg(feature = "verify")]
        let mut state = Control::new(updater, DfuAttributes::CAN_DOWNLOAD, ResetImmediate, PUBLIC_SIGNING_KEY);

        let mut builder = Builder::new(
            driver,
            config,
            &mut config_descriptor,
            &mut bos_descriptor,
            &mut [],
            &mut control_buf,
        );

        // We add MSOS headers so that the device automatically gets assigned the WinUSB driver on Windows.
        // Otherwise users need to do this manually using a tool like Zadig.
        //
        // It seems these always need to be at added at the device level for this to work and for
        // composite devices they also need to be added on the function level (as shown later).
        //
        builder.msos_descriptor(msos::windows_version::WIN8_1, 2);
        builder.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));
        builder.msos_feature(msos::RegistryPropertyFeatureDescriptor::new(
            "DeviceInterfaceGUIDs",
            msos::PropertyData::RegMultiSz(DEVICE_INTERFACE_GUIDS),
        ));

        usb_dfu::<_, _, _, _, 4096>(&mut builder, &mut state, |func| {
            // You likely don't have to add these function level headers if your USB device is not composite
            // (i.e. if your device does not expose another interface in addition to DFU)
            func.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));
            func.msos_feature(msos::RegistryPropertyFeatureDescriptor::new(
                "DeviceInterfaceGUIDs",
                msos::PropertyData::RegMultiSz(DEVICE_INTERFACE_GUIDS),
            ));
        });

        let mut dev = builder.build();
        embassy_futures::block_on(dev.run());
    }

    led.set_high();
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
