const stateText = document.getElementById('websocket-state');

function setState(state) {
    console.log(state);
    stateText.textContent = state;
}

function startWebsocket() {
    try {
        setState('Verbindung für Echtzeitaktualisierungen wird hergestellt …');
        const sock = new WebSocket("wss://status.gefolge.org/websocket");
        sock.onopen = () => {
            setState('Status wird live aktualisiert.');
        };
        sock.onmessage = (event) => {
            const payload = JSON.parse(event.data);
            switch (payload.type) {
                case 'ping': {
                    break;
                }
                case 'change': {
                    if ('gefolgeWebRunning' in payload) {
                        const gefolgeWebCurrent = document.createElement('a');
                        gefolgeWebCurrent.setAttribute('href', `https://github.com/dasgefolge/gefolge.org/commit/${payload.gefolgeWebRunning}`);
                        gefolgeWebCurrent.appendChild(document.createTextNode(payload.gefolgeWebRunning.slice(0, 7)));
                        document.getElementById('gefolge-web-current').replaceChildren(gefolgeWebCurrent);
                        document.getElementById('gefolge-web-history').setAttribute('href', `https://github.com/dasgefolge/gefolge.org/commits/${payload.gefolgeWebRunning}`);
                    }
                    if ('gefolgeWebFuture' in payload) {
                        if (payload.gefolgeWebFuture.length == 0) {
                            document.getElementById('gefolge-web-future-empty').removeAttribute('style');
                            document.getElementById('gefolge-web-future-nonempty').setAttribute('style', 'display: none;');
                        } else {
                            document.getElementById('gefolge-web-future-empty').setAttribute('style', 'display: none;');
                            document.getElementById('gefolge-web-future-nonempty').removeAttribute('style');
                            const gefolgeWebFutureChildren = payload.gefolgeWebFuture
                                .map((commit) => {
                                    const row = document.createElement('tr');
                                    const hashCell = document.createElement('td');
                                    const hash = document.createElement('code');
                                    const hashLink = document.createElement('a');
                                    hashLink.setAttribute('href', `https://github.com/dasgefolge/gefolge.org/commit/${commit.commitHash}`);
                                    hashLink.appendChild(document.createTextNode(commit.commitHash.slice(0, 7)));
                                    hash.appendChild(hashLink);
                                    hashCell.appendChild(hash);
                                    row.appendChild(hashCell);
                                    const msgCell = document.createElement('td');
                                    msgCell.appendChild(document.createTextNode(commit.commitMsg));
                                    row.appendChild(msgCell);
                                    const statusCell = document.createElement('td');
                                    switch (commit.status.type) {
                                        case 'pending': statusCell.appendChild(document.createTextNode('wartet auf Abschluss anderer Builds')); break;
                                        case 'bundled': statusCell.appendChild(document.createTextNode('übersprungen (mit dem nächsten Commit gebündelt)')); break;
                                        case 'deploy': statusCell.appendChild(document.createTextNode('wird installiert')); break;
                                    }
                                    row.appendChild(statusCell);
                                    return row;
                                });
                            document.getElementById('gefolge-web-future-tbody').replaceChildren(...gefolgeWebFutureChildren);
                        }
                    }
                    if ('statusFuture' in payload) {
                        if (payload.statusFuture.length == 0) {
                            document.getElementById('status-future-empty').removeAttribute('style');
                            document.getElementById('status-future-nonempty').setAttribute('style', 'display: none;');
                        } else {
                            document.getElementById('status-future-empty').setAttribute('style', 'display: none;');
                            document.getElementById('status-future-nonempty').removeAttribute('style');
                            const statusFutureChildren = payload.statusFuture
                                .map((commit) => {
                                    const row = document.createElement('tr');
                                    const hashCell = document.createElement('td');
                                    const hash = document.createElement('code');
                                    hash.appendChild(document.createTextNode(commit.commitHash.slice(0, 7)));
                                    hashCell.appendChild(hash);
                                    row.appendChild(hashCell);
                                    const msgCell = document.createElement('td');
                                    msgCell.appendChild(document.createTextNode(commit.commitMsg));
                                    row.appendChild(msgCell);
                                    const statusCell = document.createElement('td');
                                    switch (commit.status.type) {
                                        case 'pending': statusCell.appendChild(document.createTextNode('wartet auf Abschluss anderer Builds')); break;
                                        case 'bundled': statusCell.appendChild(document.createTextNode('übersprungen (mit dem nächsten Commit gebündelt)')); break;
                                        case 'build': statusCell.appendChild(document.createTextNode('NixOS-Konfiguration wird aktiviert')); break;
                                    }
                                    row.appendChild(statusCell);
                                    return row;
                                });
                            document.getElementById('status-future-tbody').replaceChildren(...statusFutureChildren);
                        }
                    }
                    break;
                }
                case 'refresh': {
                    // status.gefolge.org is about to restart to update, schedule a reload
                    document.getElementById('status-future-empty').removeAttribute('style');
                    document.getElementById('status-future-empty').replaceChildren(document.createTextNode('status.gefolge.org wird neu gestartet …'));
                    document.getElementById('status-future-nonempty').setAttribute('style', 'display: none;');
                    setTimeout(window.location.reload.bind(window.location), 5000);
                    break;
                }
            }
        };
        sock.onerror = (event) => {
            console.log(`WebSocket-Fehler: ${event}`);
            throw event;
        }
        sock.onclose = () => {
            setState('Verbindung für Echtzeitaktualisierungen unterbrochen, neue Verbindung wird hergestellt …');
            setTimeout(startWebsocket, 1000);
        }
    } catch (e) {
        setState(`Fehler bei der Echtzeitaktualisierung: ${e}. Bitte melde diesen Fehler im #dev.`);
    }
}

startWebsocket();
