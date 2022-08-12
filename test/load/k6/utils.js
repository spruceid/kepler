import { check } from 'k6';
import http from 'k6/http';

export const kepler = __ENV.KEPLER || "http://127.0.0.1:8000";
export const signer = __ENV.SIGNER || "http://127.0.0.1:3000";

export function setup_orbit(kepler, signer) {
    let peer_id = http.get(`${kepler}/peer/generate`).body;
    let orbit_creation = http.post(`${signer}/orbit`,
        JSON.stringify({ peer_id }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    console.log(`Orbit Creation: ${orbit_creation}`);
    let res = http.post(`${kepler}/delegate`,
        null,
        {
            headers: orbit_creation,
        });
    check(res, {
        'orbit creation is succesful': (r) => r.status === 200,
    });
    console.log(`Orbit creation (${res.headers["Spruce-Trace-Id"]}) -> ${res.status}`);
    let session_delegation = http.post(`${signer}/session/create`).json();
    res = http.post(`${kepler}/delegate`,
        null,
        {
            headers: session_delegation,
        });
    check(res, {
        'session delegation is succesful': (r) => r.status === 200,
    });

    console.log(`Session delegation (${res.headers["Spruce-Trace-Id"]}) -> ${res.status}`);
}
