# What Ports Work?

This is a server that listens on all TCP and UDP ports, allowing you to test which outbound ports are blocked or not. This is the reverse of a port forwarding test, which tests which inbound ports are blocked. Often firewalls on business or guest WiFi networks will block ports other than 53, 80, and 443. This tool will help you detect these blocks.

There are multiple ways to use [whatports.work](http://whatports.work) (replacing 1234 with your desired port number):

- Using the JavaScript test on the website.
- Changing the port number in the URL, e.g. http://whatports.work:1234, HTTPS is currently unsupported.
- Using telnet or similar command line tools, e.g. telnet whatports.work 1234
  - netcat can be used for UDP testing, e.g. echo | nc -u whatports.work 1234
- Using curl, e.g. curl http://whatports.work:1234/raw
- Using nmap, e.g. nmap -p 1234 whatports.work

This project was inspired by [portquiz.net](http://portquiz.net), and aims to be a more modern and robust implementation.
