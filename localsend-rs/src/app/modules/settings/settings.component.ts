import { Component } from '@angular/core';
import { InputTextModule } from 'primeng/inputtext';
import { DropdownModule } from 'primeng/dropdown';
import { FormsModule } from '@angular/forms';
@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [FormsModule, InputTextModule, DropdownModule],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.css'
})
export class SettingsComponent {
  device_name = 'Good Tomato';
  multicast_address = '224.0.0.167';
  port = 53317;
}
