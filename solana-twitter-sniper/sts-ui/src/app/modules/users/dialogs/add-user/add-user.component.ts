import { AfterViewInit, Component, inject } from '@angular/core';
import { FormGroup, FormControl, Validators } from '@angular/forms';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { SharedModule } from '../../../../shared/shared.module';
import { CommonModule } from '@angular/common';

@Component({
  selector: 'app-add-user',
  imports: [CommonModule, SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './add-user.component.html',
  styleUrl: './add-user.component.scss'
})
export class AddUserComponent implements AfterViewInit {
  groups = [
    {
      id: 3,
      name: 'Trader',
    }
  ]
  addUserForm = new FormGroup({
    address: new FormControl('', [Validators.required, Validators.minLength(42), Validators.maxLength(42)]),
    group_id: new FormControl(3, [Validators.required]),
  })

  readonly dialogRef = inject(MatDialogRef<AddUserComponent>);
  ngAfterViewInit(): void {

  }

  public addUser() {
    let { address, group_id } = this.addUserForm.value;
    let payload = {
      address,
      group_id
    }
    this.dialogRef.close(payload)
  }
}
