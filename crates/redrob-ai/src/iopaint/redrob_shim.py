# SPDX-License-Identifier: MIT OR Apache-2.0
"""Redrob Canvas starts IOPaint 1.6.0 through this file instead of the `iopaint` entry point.

It runs IOPaint's own command line unchanged (every argument after the script name is handed to
`iopaint`), and before the server comes up it adds four routes to the API object so pixels can
cross as raw files on this machine instead of base64 PNGs inside JSON:

    GET  /api/v1/redrob/transport           -> {"file": true, "exchange_dir": "..."}
    POST /api/v1/redrob/inpaint             image_path, mask_path, output_path, width, height
                                            + every InpaintRequest field (prompt, sd_steps, ...)
    POST /api/v1/redrob/plugin_gen_image    name, image_path (RGBA), output_path, width, height, scale
    POST /api/v1/redrob/plugin_gen_mask     name, image_path (RGBA), output_path, width, height, clicks

Files are raw interleaved bytes, row-major, top row first: RGB (3 bytes/px) for an inpaint
input and output, RGBA (4 bytes/px) for a plugin input, L8 for a mask. The responses carry the
result's width, height and channel count. Every path must lie inside the exchange directory
given by the REDROB_AI_EXCHANGE_DIR environment variable; anything else is refused, so the
loopback server can read and write nothing but that folder.

If the patch cannot be applied (another IOPaint layout), the server still starts as plain IOPaint
and the client keeps using base64.
"""

import os
import sys
import time
from pathlib import Path
from typing import List, Optional


def _exchange_dir() -> Optional[Path]:
    raw = os.environ.get("REDROB_AI_EXCHANGE_DIR")
    if not raw:
        return None
    return Path(raw).expanduser().resolve()


def _install_routes(api, exchange: Path):
    import cv2
    import numpy as np
    from fastapi import HTTPException
    from loguru import logger
    from pydantic import BaseModel, Field

    from iopaint.helper import gen_frontend_mask
    from iopaint.model.utils import torch_gc
    from iopaint.schema import InpaintRequest, RunPluginRequest

    def checked(path: str) -> Path:
        p = Path(path).expanduser().resolve()
        try:
            p.relative_to(exchange)
        except ValueError:
            raise HTTPException(400, detail="path outside the exchange directory")
        return p

    def read_raw(path: str, width: int, height: int, channels: int) -> np.ndarray:
        p = checked(path)
        expected = width * height * channels
        try:
            data = np.fromfile(p, dtype=np.uint8)
        except OSError as e:
            raise HTTPException(400, detail=f"cannot read {p.name}: {e}")
        if data.size != expected:
            raise HTTPException(
                400,
                detail=f"{p.name} holds {data.size} bytes, not {width}x{height}x{channels}",
            )
        if channels == 1:
            return data.reshape(height, width)
        return data.reshape(height, width, channels)

    def write_raw(path: str, array: np.ndarray):
        p = checked(path)
        np.ascontiguousarray(array, dtype=np.uint8).tofile(p)

    class RedrobInpaintRequest(InpaintRequest):
        image_path: str = Field(..., description="raw RGB file")
        mask_path: str = Field(..., description="raw L8 file")
        output_path: str = Field(..., description="where the raw RGB result goes")
        width: int
        height: int

    class RedrobPluginRequest(BaseModel):
        name: str
        image_path: str = Field(..., description="raw RGBA file")
        output_path: str
        width: int
        height: int
        clicks: List[List[int]] = Field([])
        scale: float = Field(2.0)

    def api_transport():
        return {"file": True, "exchange_dir": str(exchange)}

    def api_inpaint(req: RedrobInpaintRequest):
        image = read_raw(req.image_path, req.width, req.height, 3)
        mask = read_raw(req.mask_path, req.width, req.height, 1)
        mask = cv2.threshold(mask, 127, 255, cv2.THRESH_BINARY)[1]
        start = time.time()
        bgr = api.model_manager(image, mask, req)
        logger.info(f"process time: {(time.time() - start) * 1000:.2f}ms (file transport)")
        torch_gc()
        rgb = cv2.cvtColor(bgr.astype(np.uint8), cv2.COLOR_BGR2RGB)
        write_raw(req.output_path, rgb)
        return {
            "width": int(rgb.shape[1]),
            "height": int(rgb.shape[0]),
            "channels": 3,
            "seed": req.sd_seed,
        }

    def plugin_for(req: RedrobPluginRequest):
        if req.name not in api.plugins:
            raise HTTPException(status_code=422, detail="Plugin not found")
        return api.plugins[req.name]

    def api_plugin_gen_image(req: RedrobPluginRequest):
        plugin = plugin_for(req)
        if not plugin.support_gen_image:
            raise HTTPException(status_code=422, detail="Plugin does not support output image")
        rgba = read_raw(req.image_path, req.width, req.height, 4)
        alpha = rgba[:, :, 3]
        rgb = cv2.cvtColor(rgba, cv2.COLOR_RGBA2RGB)
        plugin_req = RunPluginRequest(name=req.name, image="", clicks=req.clicks, scale=req.scale)
        result = plugin.gen_image(rgb, plugin_req)
        torch_gc()
        if result.shape[2] == 4:
            out = result
        else:
            out = cv2.cvtColor(result, cv2.COLOR_BGR2RGB)
            if alpha.shape[:2] != out.shape[:2]:
                alpha = cv2.resize(alpha, dsize=(out.shape[1], out.shape[0]))
            out = np.concatenate((out, alpha[:, :, np.newaxis]), axis=-1)
        write_raw(req.output_path, out)
        return {"width": int(out.shape[1]), "height": int(out.shape[0]), "channels": 4}

    def api_plugin_gen_mask(req: RedrobPluginRequest):
        plugin = plugin_for(req)
        if not plugin.support_gen_mask:
            raise HTTPException(status_code=422, detail="Plugin does not support output image")
        rgba = read_raw(req.image_path, req.width, req.height, 4)
        rgb = cv2.cvtColor(rgba, cv2.COLOR_RGBA2RGB)
        plugin_req = RunPluginRequest(name=req.name, image="", clicks=req.clicks, scale=req.scale)
        painted = gen_frontend_mask(plugin.gen_mask(rgb, plugin_req))
        torch_gc()
        mask = np.where(painted[:, :, 3] > 0, 255, 0).astype(np.uint8)
        write_raw(req.output_path, mask)
        return {"width": int(mask.shape[1]), "height": int(mask.shape[0]), "channels": 1}

    api.add_api_route("/api/v1/redrob/transport", api_transport, methods=["GET"])
    api.add_api_route("/api/v1/redrob/inpaint", api_inpaint, methods=["POST"])
    api.add_api_route("/api/v1/redrob/plugin_gen_image", api_plugin_gen_image, methods=["POST"])
    api.add_api_route("/api/v1/redrob/plugin_gen_mask", api_plugin_gen_mask, methods=["POST"])
    logger.info(f"Redrob file transport on, exchange dir {exchange}")


def _patch():
    exchange = _exchange_dir()
    if exchange is None:
        return
    exchange.mkdir(parents=True, exist_ok=True)
    import iopaint.api as api_module

    original_init = api_module.Api.__init__

    def patched_init(self, app, config):
        original_init(self, app, config)
        try:
            _install_routes(self, exchange)
        except Exception as e:  # noqa: BLE001 - the server must still come up
            print(f"redrob shim: file transport unavailable: {e}", file=sys.stderr)

    api_module.Api.__init__ = patched_init


def main():
    try:
        _patch()
    except Exception as e:  # noqa: BLE001
        print(f"redrob shim: not patched, plain IOPaint: {e}", file=sys.stderr)
    from iopaint import entry_point

    sys.argv = ["iopaint"] + sys.argv[1:]
    entry_point()


if __name__ == "__main__":
    main()
