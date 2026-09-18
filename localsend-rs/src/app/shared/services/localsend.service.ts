import { Injectable } from '@angular/core';
import { Subject } from 'rxjs';
type StorageModel = {
  device_name: string,
  multicast_address: string,
  port: number
};

const DEFAULT_DEVICE_NAME = 'Good Tomato';
const DEFAULT_MULTICAST_ADDRESS = '224.0.0.167';
const DEFAULT_PORT = 53317;

@Injectable({
  providedIn: 'root'
})
export class LocalSendService {
  $updateValues = new Subject<StorageModel>();

  constructor() {
  }

  init() {
    let updated = false;
    if (!localStorage.getItem('device_name')) {
      localStorage.setItem('device_name', DEFAULT_DEVICE_NAME);
      updated = true;
    }
    if (!localStorage.getItem('multicast_address')) {
      localStorage.setItem('multicast_address', DEFAULT_MULTICAST_ADDRESS);
      updated = true;
    }
    if (!localStorage.getItem('port')) {
      localStorage.setItem('port', DEFAULT_PORT.toString());
      updated = true;
    }
    if (updated) {
      let values = this.getValues() as StorageModel;
      this.$updateValues.next(values);
    }
  }

  getValues(): StorageModel {
    let values: { [key: string]: string | number } = {};
    try {
      values['device_name'] = localStorage.getItem('device_name') ?? DEFAULT_DEVICE_NAME;
    } catch (err) {
      values['device_name'] = DEFAULT_DEVICE_NAME;
    }

    try {
      values['multicast_address'] = localStorage.getItem('multicast_address') ?? DEFAULT_MULTICAST_ADDRESS;
    } catch (err) {
      values['multicast_address'] = DEFAULT_MULTICAST_ADDRESS;
    }

    try {
      values['port'] = +(localStorage.getItem('port') ?? DEFAULT_PORT);
    } catch (err) {
      values['port'] = DEFAULT_PORT;
    }
    return values as StorageModel;
  }
}
