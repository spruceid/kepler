import http from 'k6/http';
import { b64encode } from 'k6/encoding';

export const kepler = __ENV.KEPLER ? __ENV.KEPLER : "http://127.0.0.1:8000";
export const signer = __ENV.SIGNER ? __ENV.SIGNER : "http://127.0.0.1:3000";

export function setup_orbit(kepler, signer) {
    let peer_id = http.get(`${kepler}/peer/generate`).body;
    let orbit_creation = http.post(`${signer}/orbit`,
        JSON.stringify({ peer_id }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).body;
    http.post(`${kepler}/delegate`,
        null,
        {
            headers: {
                'Authorization': b64encode(orbit_creation),
            }
        });
    let session_delegation = http.post(`${signer}/session/create`).body;
    http.post(`${kepler}/delegate`,
        null,
        {
            headers: {
                'Authorization': b64encode(session_delegation),
            }
        });
}
