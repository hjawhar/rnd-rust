# Monitored
### Docker & PM2 instances checker
<p>
Using russh client, the application connects to specified servers and data collection of the available pm2 and docker instances.<br>
The following was initially designed to check running services on multiple servers simultaneously, rather than one-by-one.
</p>

## Usage
```
$ cargo run host username port pk_path
```

![Screenshot](https://github.com/hjawhar/monitored/blob/master/screenshots/screenshot_1.png) 
