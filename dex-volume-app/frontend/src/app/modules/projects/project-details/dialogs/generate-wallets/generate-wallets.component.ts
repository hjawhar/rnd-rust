import { AfterViewInit, Component, inject } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { FormControl, FormGroup, Validators } from '@angular/forms';
import { SharedModule } from '../../../../../shared/shared.module';

@Component({
  selector: 'app-generate-wallets',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './generate-wallets.component.html',
  styleUrl: './generate-wallets.component.scss'
})
export class GenerateWalletsComponent implements AfterViewInit {
  generateForm = new FormGroup({
    value: new FormControl(1, [Validators.required, Validators.min(1)])
  })
  readonly dialogRef = inject(MatDialogRef<GenerateWalletsComponent>);
  ngAfterViewInit(): void {

  }

  public generate() {
    let { value } = this.generateForm.value;
    let payload = {
      value: +value!,
    }
    this.dialogRef.close(payload)
  }
}
