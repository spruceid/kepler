import { check } from 'k6';
import http from 'k6/http';
import { b64encode } from 'k6/encoding';
import {
    randomString,
} from 'https://jslib.k6.io/k6-utils/1.3.0/index.js';

import { setup_orbit, kepler, signer } from './utils.js';

const key = randomString(15);
export function setup() {
    setup_orbit(kepler, signer);

    let put_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: key, action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).body;
    http.post(`${kepler}/invoke`,
        JSON.stringify({ test: "data" }),
        {
            headers: {
                'Content-Type': 'application/json',
                'Authorization': b64encode(put_invocation),
            },
        }
    );
}

export default function() {
    let get_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: key, action: "get" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).body;
    let res = http.post(`${kepler}/invoke`,
        "",
        {
            headers: {
                'Content-Type': 'application/json',
                'Authorization': b64encode(get_invocation),
            },
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);
}
