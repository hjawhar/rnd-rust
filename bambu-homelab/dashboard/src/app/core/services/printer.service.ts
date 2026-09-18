import { Injectable, inject } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable } from 'rxjs';
import { PrinterWithStatus, PrinterConfig } from '../models/printer.model';
import { CommandRequest } from '../models/command.model';

@Injectable({ providedIn: 'root' })
export class PrinterService {
  private http = inject(HttpClient);
  private baseUrl = '/api';

  listPrinters(): Observable<PrinterWithStatus[]> {
    return this.http.get<PrinterWithStatus[]>(`${this.baseUrl}/printers`);
  }

  getPrinter(id: string): Observable<PrinterWithStatus> {
    return this.http.get<PrinterWithStatus>(`${this.baseUrl}/printers/${id}`);
  }

  addPrinter(printer: { ip: string; serial: string; access_code: string; name: string; model?: string }): Observable<PrinterConfig> {
    return this.http.post<PrinterConfig>(`${this.baseUrl}/printers`, printer);
  }

  removePrinter(id: string): Observable<void> {
    return this.http.delete<void>(`${this.baseUrl}/printers/${id}`);
  }

  sendCommand(printerId: string, command: CommandRequest): Observable<void> {
    return this.http.post<void>(`${this.baseUrl}/printers/${printerId}/command`, command);
  }

  getStats(printerId: string): Observable<any> {
    return this.http.get(`${this.baseUrl}/printers/${printerId}/stats`);
  }

  listUsers(): Observable<{id: string; username: string; role: string}[]> {
    return this.http.get<{id: string; username: string; role: string}[]>(`${this.baseUrl}/users`);
  }

  createUser(req: {username: string; password: string; role?: string}): Observable<void> {
    return this.http.post<void>(`${this.baseUrl}/users`, req);
  }

  deleteUser(id: string): Observable<void> {
    return this.http.delete<void>(`${this.baseUrl}/users/${id}`);
  }

  // Assignments
  listAssignments(printerId: string): Observable<{user_id: string; username: string; assigned_at: string}[]> {
    return this.http.get<{user_id: string; username: string; assigned_at: string}[]>(
      `${this.baseUrl}/printers/${printerId}/assignments`
    );
  }

  assignPrinter(printerId: string, userId: string): Observable<void> {
    return this.http.post<void>(`${this.baseUrl}/printers/${printerId}/assignments`, { user_id: userId });
  }

  unassignPrinter(printerId: string, userId: string): Observable<void> {
    return this.http.delete<void>(`${this.baseUrl}/printers/${printerId}/assignments/${userId}`);
  }
}