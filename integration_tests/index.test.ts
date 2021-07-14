import { Kepler, authenticator } from 'kepler-sdk';
import { DAppClient } from '@airgap/beacon-sdk';
import { InMemorySigner } from '@taquito/signer';
import { b58cencode, prefix } from "@taquito/utils";
import { DockerComposeEnvironment, StartedDockerComposeEnvironment } from "testcontainers";
import { ContractClient } from "orbit-manifest";

import path = require('path');
const fetch = require('node-fetch');
import crypto = require('crypto');

const admin = 'edsk3QoqBuvdamxouPhin7swCvkQNgq4jP5KZPbwWNnwdZpSpJiEbq';

const buildContext = path.resolve(__dirname, '..');
const composeFile = 'sandbox.yml';
const kepler1Port = 8000;
const kepler2Port = 9000;

const genClient = async (): Promise<DAppClient> => {
    const ims = new InMemorySigner(b58cencode(
        crypto.randomBytes(32),
        prefix.edsk2
    ));
    // @ts-ignore
    return {
        // @ts-ignore
        getActiveAccount: async () => ({ publicKey: await ims.publicKey(), address: await ims.publicKeyHash() }),
        // @ts-ignore
        requestSignPayload: async ({ payload }) => ({ signature: await ims.sign(payload).then(res => res.prefixSig) })
    }
}

const create = async (hostURL: string = 'http://localhost:8000', peerURL: string = 'http://localhost:9000') => {
    const authn = await authenticator(await genClient(), 'test');
    const kepler1 = new Kepler(hostURL, authn);
    const res0 = await kepler1.createOrbit({ hi: 'there' });
    expect(res0.status).toEqual(200);
    const cid = await res0.text();
    const res1 = await kepler1.resolve(cid);
    expect(res1.status).toEqual(200);
    await expect(res1.json()).resolves.toEqual({ hi: 'there' });

    const kepler2 = new Kepler(peerURL, authn)
    const res2 = await kepler2.resolve(cid)
    expect(res2.status).toEqual(200);
    return await expect(res2.json()).resolves.toEqual({ hi: 'there' });
}

describe('Kepler Integration Tests', () => {
    let environment: StartedDockerComposeEnvironment;
    let kepler1Url: string;
    let kepler2Url: string;

    beforeAll(async () => {
        // environment = await new DockerComposeEnvironment(buildContext, composeFile).up()
        // let kepler1Host = environment.getContainer("kepler-1").getHost();
        // let kepler2Host = environment.getContainer("kepler-2").getHost();
        const kepler1Host = "localhost";
        const kepler2Host = "localhost";

        kepler1Url = "http://" + kepler1Host + ":" + kepler1Port;
        kepler2Url = "http://" + kepler2Host + ":" + kepler2Port;

        // @ts-ignore
        const { id: k1 } = await fetch(kepler1Url + "/hostInfo").then(async r => await r.json());
        // @ts-ignore
        const { id: k2 } = await fetch(kepler2Url + "/hostInfo").then(async r => await r.json());

        console.log(k1, k2);

        // deploy contract
        const client = new ContractClient({
            tzktBase: "http://localhost:5000",
            nodeURL: "http://localhost:8732",
            contractType: "",
            signer: {
                type: 'secret',
                secret: admin
            }
        });

        const contract = await client.originate({
            hosts: {
                [k1]: [kepler1Host + ":" + (kepler1Port + 1)],
                [k2]: [kepler2Host + ":" + (kepler2Port + 1)],
            },
            admins: []
        });

        console.log(contract)
        // 10 minute time limit for building the container
    }, 600000)

    afterAll(async () => {
        // await environment.down()
        // 1 minute time limit for stopping the container
    }, 60000)

    it('sequential load', async () => {
        const len = 100
        for (let i = 0; i < len; i++) {
            await create(kepler1Url)
        }
    }, 60000)

    // it('concurrent load', async () => {
    //     const len = 1000
    //     const p = []
    //     for (let i = 0; i < len; i++) {
    //         p.push(create())
    //     }
    //     await Promise.all(p)
    // })
    //     const cids = await orbit.put(json1, json2);
    //     console.log(cids)

    //     // await expect(orbit.get(cid)).resolves.toEqual(json)
    //     // return await expect(orbit.del(cid)).resolves.not.toThrow()
    // })
})
