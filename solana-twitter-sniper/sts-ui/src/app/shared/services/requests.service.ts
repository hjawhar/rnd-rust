import { Injectable } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { Observable } from 'rxjs';
import { getJwtToken } from '../utils';
@Injectable({
  providedIn: 'root'
})
export class RequestsService {

  httpOptions = {
    headers: new HttpHeaders({
      'Content-Type': 'application/json',
      Authorization: `Bearer ${getJwtToken()}`
    })
  };

  constructor(private http: HttpClient) {
  }

  public post(url: string, body: any): Observable<any> {
    return this.http.post(url, body);
  }

  public get(url: string): Observable<any> {
    return this.http.get(url);
  }

  public edit(url: string, body: any): Observable<any> {
    return this.http.put(url, body);
  }

  public delete(url: string): Observable<any> {
    return this.http.delete(url);
  }
}
