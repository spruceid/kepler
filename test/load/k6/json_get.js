import { check } from 'k6';
import http from 'k6/http';
import {
    randomString,
} from 'https://jslib.k6.io/k6-utils/1.3.0/index.js';

import { setup_orbit, kepler, signer } from './utils.js';

export const options = {
    scenarios: {
        constant_request_rate: {
            executor: 'constant-arrival-rate',
            rate: 10,
            timeUnit: '1s',
            duration: '5s',
            preAllocatedVUs: 25,
        },
    },
};

export function setup() {
    setup_orbit(kepler, signer);

    const key = randomString(15);
    let put_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: key, action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    put_invocation['Content-Type'] = 'application/json';
    http.post(`${kepler}/invoke`,
        JSON.stringify({ test: "data" }),
        {
            headers: put_invocation,
        }
    );
    return { key };
}

export default function(data) {
    let start_now = Date.now();
    let get_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: data.key, action: "get" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    get_invocation['Content-Type'] = 'application/json';
    let signer_now = Date.now();
    let res = http.post(`${kepler}/invoke`,
        "",
        {
            headers: get_invocation,
        }
    );
    let invoke_now = Date.now();
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`${res.headers["Spruce-Trace-Id"]} -> ${res.status} [${signer_now - start_now}ms + ${invoke_now - signer_now}ms]`);
}

export function teardown(data) {
    let del_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: data.key, action: "del" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    del_invocation['Content-Type'] = 'application/json';
    let res = http.post(`${kepler}/invoke`,
        "",
        {
            headers: del_invocation
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
}
