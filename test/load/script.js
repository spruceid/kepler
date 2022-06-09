import http from 'k6/http';
// import { sleep } from 'k6';
import { b64encode } from 'k6/encoding';

const kepler = "http://127.0.0.1:8000";

const peer_id = http.get(`${kepler}/peer/generate`).body;
http.post(`${kepler}/delegate`,
    null,
    {
        headers: {
            'Authorization': b64encode(orbit_creation),
        }
    });

export default function() {
    let put_invocation = http.post('http://127.0.0.1:3000/invoke',
        JSON.stringify({ name: "test", action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).body;
    http.post(`${kepler}/invoke`,
        JSON.stringify({ name: "test", action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
                'Authorization': b64encode(put_invocation),
            },
        }
    );
    // sleep(1);
}
