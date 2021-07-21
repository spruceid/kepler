import { Kepler, authenticator } from 'kepler-sdk';
import { DAppClient } from '@airgap/beacon-sdk';
import { InMemorySigner } from '@taquito/signer';
import { b58cencode, prefix } from "@taquito/utils";
import { DockerComposeEnvironment, StartedDockerComposeEnvironment } from "testcontainers";
import { ContractClient } from "orbit-manifest";

import path = require('path');
const fetch = require('node-fetch');
import crypto = require('crypto');

const secrets = ['edsk3QoqBuvdamxouPhin7swCvkQNgq4jP5KZPbwWNnwdZpSpJiEbq', 'edsk3RFfvaFaxbHx8BMtEW1rKQcPtDML3LXjNqMNLCzC3wLC1bWbAt'];

const buildContext = path.resolve(__dirname, '..');
const composeFile = 'sandbox.yml';
const kepler1Port = 8000;
const kepler2Port = 9000;

const genClient = async (secret: string = b58cencode(
    crypto.randomBytes(32),
    prefix.edsk2
)): Promise<DAppClient> => {
    const ims = new InMemorySigner(secret);
    // @ts-ignore
    return {
        // @ts-ignore
        getActiveAccount: async () => ({ publicKey: await ims.publicKey(), address: await ims.publicKeyHash() }),
        // @ts-ignore
        requestSignPayload: async ({ payload }) => ({ signature: await ims.sign(payload).then(res => res.prefixSig) })
    }
}

type HostInfo = {
    id: string,
    port: number,
    url: string
}

const create = async (
    secret: string,
    [host, ...peers]: HostInfo[],
) => {
    // get IDs and multiaddrs of all kepler nodes
    const hostInfo = await Promise.all([host, ...peers].map(async host => {
        const api = "http://" + host.url + ":" + host.port;
        // @ts-ignore
        const key: string = await fetch(api + "/hostInfo").then(r => r.json()).then(({ id }) => id)
        return { [key]: ["/ip4/0.0.0.0/tcp/" + (host.port + 1)] }
    })).then(hosts => hosts.reduce((h, acc) => ({ ...h, ...acc })));

    // deploy contract
    const client = new ContractClient({
        tzktBase: "http://localhost:5000",
        nodeURL: "http://localhost:8732",
        contractType: "",
        signer: {
            type: 'secret',
            secret
        }
    });

    const contract = await client.originate({
        hosts: hostInfo,
        admins: []
    });

    // wait for tzkt to index the contract
    await new Promise(resolve => setTimeout(resolve, 1000));

    // @ts-ignore
    const authn = await authenticator(await genClient(secret), 'test', contract);

    const hostClient = new Kepler("http://" + host.url + ":" + host.port, authn);
    const peerClients = peers.map(peer => new Kepler("http://" + peer.url + ":" + peer.port, authn));

    const hostContent = { hi: "there" };
    const peerContent = { dummy: "data" };

    const res0 = await hostClient.createOrbit(hostContent)
    const cid = await res0.text();
    expect(res0.status).toEqual(200)
    const res1 = await hostClient.resolve(cid);
    expect(res1.status).toEqual(200);
    await expect(res1.json()).resolves.toEqual(hostContent);

    await Promise.all(peerClients.map(async peerClient => {
        const createRes = await peerClient.createOrbit(peerContent);
        expect(createRes.status).toEqual(200);

        const getRes = await peerClient.resolve(cid);
        expect(getRes.status).toEqual(200);
        return await expect(getRes.json()).resolves.toEqual(hostContent);
    }))
}

describe('Kepler Integration Tests', () => {
    let environment: StartedDockerComposeEnvironment;
    let kepler1: HostInfo;
    let kepler2: HostInfo;

    beforeAll(async () => {
        // environment = await new DockerComposeEnvironment(buildContext, composeFile).up()
        // let kepler1Host = environment.getContainer("kepler-1").getHost();
        // let kepler2Host = environment.getContainer("kepler-2").getHost();
        const kepler1Host = "localhost";
        const kepler2Host = "localhost";

        // @ts-ignore
        const { id: k1 } = await fetch("http://" + kepler1Host + ":8000" + "/hostInfo").then(async r => await r.json());
        // @ts-ignore
        const { id: k2 } = await fetch("http://" + kepler2Host + ":8000" + "/hostInfo").then(async r => await r.json());

        kepler1 = {
            url: kepler1Host,
            port: 8000,
            id: k1
        };
        kepler2 = {
            url: kepler2Host,
            port: 9000,
            id: k2
        };
        // 10 minute time limit for building the container
    }, 600000)

    afterAll(async () => {
        // await environment.down()
        // 1 minute time limit for stopping the container
    }, 60000)

    it('concurrent load', async () => {
        await Promise.all(secrets.map(secret => create(secret, [kepler1, kepler2])))
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
