// Restricted ports list from https://fetch.spec.whatwg.org/#port-blocking
// These ports are blocked by browsers for security reasons, so we won't include them in the full scan to avoid confusion.
let restrictedPorts = [
    0,      // Not in Fetch Spec.
    1,      // tcpmux
    7,      // echo
    9,      // discard
    11,     // systat
    13,     // daytime
    15,     // netstat
    17,     // qotd
    19,     // chargen
    20,     // ftp data
    21,     // ftp access
    22,     // ssh
    23,     // telnet
    25,     // smtp
    37,     // time
    42,     // name
    43,     // nicname
    53,     // domain
    69,     // tftp
    77,     // priv-rjs
    79,     // finger
    87,     // ttylink
    95,     // supdup
    101,    // hostriame
    102,    // iso-tsap
    103,    // gppitnp
    104,    // acr-nema
    109,    // pop2
    110,    // pop3
    111,    // sunrpc
    113,    // auth
    115,    // sftp
    117,    // uucp-path
    119,    // nntp
    123,    // NTP
    135,    // loc-srv /epmap
    137,    // netbios
    139,    // netbios
    143,    // imap2
    161,    // snmp
    179,    // BGP
    389,    // ldap
    427,    // SLP (Also used by Apple Filing Protocol)
    465,    // smtp+ssl
    512,    // print / exec
    513,    // login
    514,    // shell
    515,    // printer
    526,    // tempo
    530,    // courier
    531,    // chat
    532,    // netnews
    540,    // uucp
    548,    // AFP (Apple Filing Protocol)
    554,    // rtsp
    556,    // remotefs
    563,    // nntp+ssl
    587,    // smtp (rfc6409)
    601,    // syslog-conn (rfc3195)
    636,    // ldap+ssl
    989,    // ftps-data
    990,    // ftps
    993,    // ldap+ssl
    995,    // pop3+ssl
    1719,   // h323gatestat
    1720,   // h323hostcall
    1723,   // pptp
    2049,   // nfs
    3659,   // apple-sasl / PasswordServer
    4045,   // lockd
    5060,   // sip
    5061,   // sips
    6000,   // X11
    6566,   // sane-port
    6665,   // Alternate IRC [Apple addition]
    6666,   // Alternate IRC [Apple addition]
    6667,   // Standard IRC [Apple addition]
    6668,   // Alternate IRC [Apple addition]
    6669,   // Alternate IRC [Apple addition]
    6697,   // IRC + TLS
    10080,  // Amanda
]

let running = false;

let blockedPorts = [];
let openPorts = [];

async function scan() {
    running = !running;
    if (!running)
        return;
    btn_scan.textContent = "Cancel Scan";
    btn_scan.classList.remove("btn-primary");
    btn_scan.classList.add("btn-secondary");
    document.getElementById("results").classList.remove("d-none");
    let output = document.getElementById("output");
    let summary = document.getElementById("summary");
    output.innerHTML = "";
    blockedPorts = [];
    openPorts = [];

    let port_list = [];
    let portlist_input = document.getElementById("portlist").value;
    if (portlist_input) {
        let parts = portlist_input.split(",");
        for (let part of parts) {
            part = part.trim();
            if (part.includes("-")) {
                let [start, end] = part.split("-").map(x => parseInt(x));
                for (let i = start; i <= end; i++) {
                    port_list.push(i);
                }
            } else {
                port_list.push(parseInt(part));
            }
        }
    }

    for (let port of port_list) {
        if (restrictedPorts.includes(port)) {
            let row = document.createElement("tr");
            row.innerHTML = `<td>${port}</td><td class="text-warning">Restricted by Browser</td>`;
            output.appendChild(row);
            continue;
        }
        try {
            await fetch(`http://${window.location.hostname}:${port}/ping`);
            openPorts.push(port);
            let row = document.createElement("tr");
            row.innerHTML = `<td>${port}</td><td class="text-success">Open</td>`;
            output.appendChild(row);
            await new Promise(resolve => setTimeout(resolve, 100));
        } catch (e) {
            blockedPorts.push(port);
            let row = document.createElement("tr");
            row.innerHTML = `<td>${port}</td><td class="text-danger">Blocked</td>`;
            output.appendChild(row);
        }
        summary.innerHTML = `Found <span class="text-success">${openPorts.length}</span> open ports, <span class="text-danger">${blockedPorts.length}</span> blocked ports.`;
        if (!running)
            break;
    }
    btn_scan.textContent = "Scan";
    btn_scan.classList.remove("btn-secondary");
    btn_scan.classList.add("btn-primary");
    running = false;
}

btn_scan = document.getElementById("scan")
btn_scan.onclick = scan
