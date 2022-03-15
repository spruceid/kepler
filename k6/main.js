import { check } from "k6";
import { SimpleKepler } from "kepler-sdk";

export default function () {
    check(true, {
        "test": (t) => t,
    })
}