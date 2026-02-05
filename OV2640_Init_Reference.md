# OV2640 Initialization Reference (Extracted from STM32 C Codebase)

This document contains the initialization logic and register configuration parameters extracted from `Drivers/BSP/ATK_MC2640/atk_mc2640.c` and `Drivers/BSP/ATK_MC2640/atk_mc2640_cfg.h`.

## 1. Initialization Sequence (`atk_mc2640_init`)

This function performs the high-level initialization: power sequence, reset, SCCB init, ID check, and loading default registers.

```c
// From atk_mc2640.c

uint8_t atk_mc2640_init(void)
{
    uint16_t mid;
    uint16_t pid;
    
    // atk_mc2640_hw_init();           /* Hardware Init (GPIOs, Clocks) */
    atk_mc2640_exit_power_down();   /* Exit Power Down Mode (PWDN=0) */
    atk_mc2640_hw_reset();          /* Hardware Reset (RST pin toggle) */
    atk_mc2640_sccb_init();         /* SCCB Interface Init */
    atk_mc2640_sw_reset();          /* Software Reset (COM7 register) */
    
    mid = atk_mc2640_get_mid();     /* Get Manufacturer ID */
    if (mid != ATK_MC2640_MID)
    {
        printf("error");
        return ATK_MC2640_ERROR;
    }
    
    pid = atk_mc2640_get_pid();     /* Get Product ID */
    if (pid != ATK_MC2640_PID)
    {
        printf("error");
        return ATK_MC2640_ERROR;
    }
    
    atk_mc2640_init_reg();          /* Initialize Registers (Loads UXGA config by default) */
    
    // ... Memory allocation and DCMI init ...
    
    printf("OK");
    return ATK_MC2640_EOK;
}
```

### Software Reset
```c
static void atk_mc2640_sw_reset(void)
{
    atk_mc2640_reg_bank_select(ATK_MC2640_REG_BANK_SENSOR);
    atk_mc2640_write_reg(ATK_MC2640_REG_SENSOR_COM7, 0x80); // COM7 = 0x80 triggers reset
    HAL_Delay(50);
}
```

### Default Register Loading (`atk_mc2640_init_reg`)
This function loads the `atk_mc2640_init_uxga_cfg` array and sets up the window size.
```c
static void atk_mc2640_init_reg(void)
{
    uint32_t cfg_index;
    uint8_t zmow;
    uint8_t zmoh;
    uint8_t zmhh;
    
    // Load UXGA Configuration
    for (cfg_index=0; cfg_index<(sizeof(atk_mc2640_init_uxga_cfg)/sizeof(atk_mc2640_init_uxga_cfg[0])); cfg_index++)
    {
        atk_mc2640_write_reg(atk_mc2640_init_uxga_cfg[cfg_index][0], atk_mc2640_init_uxga_cfg[cfg_index][1]);
    }
    
    // ... Calculate width/height from DSP registers ...
}
```

## 2. Hardware Interface Configuration (`atk_mc2640_dcmi.c`)

This section details the DCMI (Digital Camera Interface) configuration used on the STM32, which defines pin polarity and clock edges.

### Pin Mappings (STM32 F429/L4)
*   **VSYNC**: PA8
*   **HREF (HSYNC)**: PA10
*   **PCLK**: PA9
*   **D0**: PC0
*   **D1**: PC1
*   **D2**: PC2
*   **D3**: PC3
*   **D4**: PC4
*   **D5**: PD5
*   **D6**: PB6
*   **D7**: PB7
*   **RST**: PA11
*   **PWDN**: IO Expander (PCF8574)

### Signal Polarity Settings
```c
void atk_mc2640_dcmi_init(void)
{
    // ...
    g_atk_mc2640_dcmi_sta.dcmi.Init.PCKPolarity         = DCMI_PCKPOLARITY_RISING; // Pixel Clock: Rising Edge
    g_atk_mc2640_dcmi_sta.dcmi.Init.VSPolarity          = DCMI_VSPOLARITY_LOW;     // VSYNC: Active Low
    g_atk_mc2640_dcmi_sta.dcmi.Init.HSPolarity          = DCMI_HSPOLARITY_LOW;     // HSYNC (HREF): Active Low
    // ...
}
```
**Note**: The register arrays do NOT explicitly configure COM10 (0x15), so the OV2640 likely outputs its default polarity (VSYNC High, HREF High). The STM32 `DCMI_VSPOLARITY_LOW` setting suggests the hardware might expect active low or there is a specific signal requirement.

## 3. Register Configuration Arrays (`atk_mc2640_cfg.h`)

These arrays contain the sequence of `{Register, Value}` pairs.

### UXGA Configuration (1600x1200, 15FPS)
Used in `atk_mc2640_init_reg`.

```c
const uint8_t atk_mc2640_init_uxga_cfg[][2] = {
    {0xFF, 0x00},
    {0x2C, 0xFF},
    {0x2E, 0xDF},
    {0xFF, 0x01},
    {0x3C, 0x32},
    {0x11, 0x00},
    {0x09, 0x02},
    {0x04, 0xA8},
    {0x13, 0xE5},
    {0x14, 0x48},
    {0x2C, 0x0C},
    {0x33, 0x78},
    {0x3A, 0x33},
    {0x3B, 0xFB},
    {0x3E, 0x00},
    {0x43, 0x11},
    {0x16, 0x10},
    {0x39, 0x92},
    {0x35, 0xDA},
    {0x22, 0x1A},
    {0x37, 0xC3},
    {0x23, 0x00},
    {0x34, 0xC0},
    {0x36, 0x1A},
    {0x06, 0x88},
    {0x07, 0xC0},
    {0x0D, 0x87},
    {0x0E, 0x41},
    {0x4C, 0x00},
    {0x48, 0x00},
    {0x5B, 0x00},
    {0x42, 0x03},
    {0x4A, 0x81},
    {0x21, 0x99},
    {0x24, 0x40},
    {0x25, 0x38},
    {0x26, 0x82},
    {0x5C, 0x00},
    {0x63, 0x00},
    {0x46, 0x00},
    {0x0C, 0x3C},
    {0x61, 0x70},
    {0x62, 0x80},
    {0x7C, 0x05},
    {0x20, 0x80},
    {0x28, 0x30},
    {0x6C, 0x00},
    {0x6D, 0x80},
    {0x6E, 0x00},
    {0x70, 0x02},
    {0x71, 0x94},
    {0x73, 0xC1},
    {0x3D, 0x34},
    {0x5A, 0x57},
    {0x12, 0x00},
    {0x17, 0x11},
    {0x18, 0x75},
    {0x19, 0x01},
    {0x1A, 0x97},
    {0x32, 0x36},
    {0x03, 0x0F},
    {0x37, 0x40},
    {0x4F, 0xCA},
    {0x50, 0xA8},
    {0x5A, 0x23},
    {0x6D, 0x00},
    {0x6D, 0x38},
    {0xFF, 0x00},
    {0xE5, 0x7F},
    {0xF9, 0xC0},
    {0x41, 0x24},
    {0xE0, 0x14},
    {0x76, 0xFF},
    {0x33, 0xA0},
    {0x42, 0x20},
    {0x43, 0x18},
    {0x4C, 0x00},
    {0x87, 0xD5},
    {0x88, 0x3F},
    {0xD7, 0x03},
    {0xD9, 0x10},
    {0xD3, 0x82},
    {0xC8, 0x08},
    {0xC9, 0x80},
    {0x7C, 0x00},
    {0x7D, 0x00},
    {0x7C, 0x03},
    {0x7D, 0x48},
    {0x7D, 0x48},
    {0x7C, 0x08},
    {0x7D, 0x20},
    {0x7D, 0x10},
    {0x7D, 0x0E},
    {0x90, 0x00},
    {0x91, 0x0E},
    {0x91, 0x1A},
    {0x91, 0x31},
    {0x91, 0x5A},
    {0x91, 0x69},
    {0x91, 0x75},
    {0x91, 0x7E},
    {0x91, 0x88},
    {0x91, 0x8F},
    {0x91, 0x96},
    {0x91, 0xA3},
    {0x91, 0xAF},
    {0x91, 0xC4},
    {0x91, 0xD7},
    {0x91, 0xE8},
    {0x91, 0x20},
    {0x92, 0x00},
    {0x93, 0x06},
    {0x93, 0xE3},
    {0x93, 0x05},
    {0x93, 0x05},
    {0x93, 0x00},
    {0x93, 0x04},
    {0x93, 0x00},
    {0x93, 0x00},
    {0x93, 0x00},
    {0x93, 0x00},
    {0x93, 0x00},
    {0x93, 0x00},
    {0x93, 0x00},
    {0x96, 0x00},
    {0x97, 0x08},
    {0x97, 0x19},
    {0x97, 0x02},
    {0x97, 0x0C},
    {0x97, 0x24},
    {0x97, 0x30},
    {0x97, 0x28},
    {0x97, 0x26},
    {0x97, 0x02},
    {0x97, 0x98},
    {0x97, 0x80},
    {0x97, 0x00},
    {0x97, 0x00},
    {0xC3, 0xEF},
    {0xA4, 0x00},
    {0xA8, 0x00},
    {0xC5, 0x11},
    {0xC6, 0x51},
    {0xBF, 0x80},
    {0xC7, 0x10},
    {0xB6, 0x66},
    {0xB8, 0xA5},
    {0xB7, 0x64},
    {0xB9, 0x7C},
    {0xB3, 0xAF},
    {0xB4, 0x97},
    {0xB5, 0xFF},
    {0xB0, 0xC5},
    {0xB1, 0x94},
    {0xB2, 0x0F},
    {0xC4, 0x5C},
    {0xC0, 0xC8},
    {0xC1, 0x96},
    {0x8C, 0x00},
    {0x86, 0x3D},
    {0x50, 0x00},
    {0x51, 0x90},
    {0x52, 0x2C},
    {0x53, 0x00},
    {0x54, 0x00},
    {0x55, 0x88},
    {0x5A, 0x90},
    {0x5B, 0x2C},
    {0x5C, 0x05},
    {0xD3, 0x02},
    {0xC3, 0xED},
    {0x7F, 0x00},
    {0xDA, 0x09},
    {0xE5, 0x1F},
    {0xE1, 0x67},
    {0xE0, 0x00},
    {0xDD, 0x7F},
    {0x05, 0x00},
};
```

### SVGA Configuration (800x600, 30FPS)
```c
const uint8_t atk_mc2640_init_svga_cfg[][2] = {
    // ... (Content similar to UXGA but optimized for SVGA)
    // [See full file for details]
    {0xFF, 0x00},
    {0x2C, 0xFF},
    // ...
    {0x05, 0x00},
};
```

### JPEG Output Configuration
These registers are written when switching to JPEG mode (`atk_mc2640_set_output_format`).

```c
const uint8_t atk_mc2640_set_jpeg_cfg[][2] = {
    {0xFF, 0x01},
    {0xE0, 0x14},
    {0xE1, 0x77},
    {0xE5, 0x1F},
    {0xD7, 0x03},
    {0xDA, 0x10},
    {0xE0, 0x00},
};
```

### YUV422 Configuration
Used as a prerequisite for JPEG mode.

```c
const uint8_t atk_mc2640_set_yuv422_cfg[][2] = {
    {0xFF, 0x00},
    {0xDA, 0x10},
    {0xD7, 0x03},
    {0xDF, 0x00},
    {0x33, 0x80},
    {0x3C, 0x40},
    {0xE1, 0x77},
    {0x00, 0x00},
};
```

### RGB565 Configuration
```c
const uint8_t atk_mc2640_set_rgb565_cfg[][2] = {
    {0xFF, 0x00},
    {0xDA, 0x09},
    {0xD7, 0x03},
    {0xDF, 0x02},
    {0x33, 0xA0},
    {0x3C, 0x00},
    {0xE1, 0x67},
    {0xFF, 0x01},
    {0xE0, 0x00},
    {0xE1, 0x00},
    {0xE5, 0x00},
    {0xD7, 0x00},
    {0xDA, 0x00},
    {0xE0, 0x00},
};
```

## 3. Mode Switching Logic

### Setting Output Format
```c
uint8_t atk_mc2640_set_output_format(atk_mc2640_output_format_t format)
{
    // ...
    case ATK_MC2640_OUTPUT_FORMAT_JPEG:
    {
        // 1. Set YUV422 first
        for (cfg_index=0; cfg_index<(sizeof(atk_mc2640_set_yuv422_cfg)/sizeof(atk_mc2640_set_yuv422_cfg[0])); cfg_index++)
        {
            atk_mc2640_write_reg(atk_mc2640_set_yuv422_cfg[cfg_index][0], atk_mc2640_set_yuv422_cfg[cfg_index][1]);
        }
        // 2. Then Set JPEG
        for (cfg_index=0; cfg_index<(sizeof(atk_mc2640_set_jpeg_cfg)/sizeof(atk_mc2640_set_jpeg_cfg[0])); cfg_index++)
        {
            atk_mc2640_write_reg(atk_mc2640_set_jpeg_cfg[cfg_index][0], atk_mc2640_set_jpeg_cfg[cfg_index][1]);
        }
        break;
    }
    // ...
}
```
