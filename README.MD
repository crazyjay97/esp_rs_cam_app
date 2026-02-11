# ESP LCD_CAM DEMO


<div align="center">
    ![][commit](https://github.com/crazyjay97/esp_rs_cam_app)
    ![][stars](https://github.com/crazyjay97/esp_rs_cam_app)
</div>


### board:
  * OV2640
  * ESP32_S3_DEVKIT

### core framework
* esp_hal (v1.0.0)
* embassy
* esp_radio


### For S3R16V
```
export ESP_HAL_CONFIG_PSRAM_MODE=octal
```

### save image stream to local file
```
curl http://IP/stream --output stream.mjpeg --max-time 20
```

### contrib
![](https://contrib.rocks/image?repo=crazyjay97/esp_rs_cam_app)
