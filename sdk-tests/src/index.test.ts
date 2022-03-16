import { SimpleKepler } from "kepler-sdk";
import { Wallet } from "ethers";
import fetch from "node-fetch";
import { Orbit } from "kepler-sdk/dist/orbit";
import { Blob } from 'buffer';

(global as any).window = { location: { hostname: "example.com" } };
(global as any).fetch = fetch;

describe("Orbit creation and access", () => {
    let orbit: Orbit;

    beforeAll(async () => {
        const wallet = Wallet.createRandom();
        wallet.getChainId = () => Promise.resolve(1);
        orbit = await (new SimpleKepler(wallet)).orbit();
    })

    it("put and get", async () => {
        await orbit.put("key", new Blob(["value"]))
            .then(() => orbit.get("key"))
            .then(value => value.text())
            .then(value => expect(value).toBe("value"));

    })
})