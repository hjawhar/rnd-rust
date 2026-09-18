import { AfterViewInit, Component } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { DeviceModel } from '../../shared/models/device.model';

@Component({
  selector: 'app-send',
  standalone: true,
  imports: [],
  templateUrl: './send.component.html',
  styleUrl: './send.component.css'
})
export class SendComponent implements AfterViewInit {
  nearby_devices: DeviceModel[] = []
  constructor() {

  }

  ngAfterViewInit(): void {
    this.get_nearby_devices();
  }

  get_nearby_devices() {
    invoke('get_nearby_devices')
      .then((message) => {
        console.log(message);
        this.nearby_devices = message as DeviceModel[];
      })
      .catch((error) => console.error(error));
  }
}
