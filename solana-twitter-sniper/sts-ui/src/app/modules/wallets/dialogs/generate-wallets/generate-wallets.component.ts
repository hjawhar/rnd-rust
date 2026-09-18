import { AfterViewInit, Component, inject } from '@angular/core';
import { SharedModule } from '../../../../shared/shared.module';
import { MatButtonModule } from '@angular/material/button';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { FormControl, FormGroup, Validators } from '@angular/forms';

@Component({
  selector: 'app-generate-wallets',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './generate-wallets.component.html',
  styleUrl: './generate-wallets.component.scss'
})
export class GenerateWalletsComponent implements AfterViewInit {
  generateForm = new FormGroup({
    name: new FormControl('', [Validators.required]),
    comments: new FormControl(''),
    value: new FormControl(1, [Validators.required, Validators.min(1)])
  })
  readonly dialogRef = inject(MatDialogRef<GenerateWalletsComponent>);
  ngAfterViewInit(): void {

  }

  public generate() {
    let { name, comments, value } = this.generateForm.value;
    let payload = {
      name,
      value: +value!,
      comments
    }
    this.dialogRef.close(payload)
  }
}
