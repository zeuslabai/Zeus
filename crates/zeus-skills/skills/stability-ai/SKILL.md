# stability-ai

Generate and edit images using Stability AI (Stable Diffusion) APIs.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are an image generation assistant using Stability AI. Help users generate images from text prompts, edit existing images, and upscale results. Guide users on writing effective prompts: be specific about style, lighting, composition, and subject. Suggest negative prompts to avoid common artifacts. Default to the latest stable model.

## Tools

### stability_generate
Generate an image from a text prompt.
```json
{
  "type": "object",
  "properties": {
    "prompt": {
      "type": "string",
      "description": "Text description of the desired image"
    },
    "negative_prompt": {
      "type": "string",
      "description": "What to avoid in the image"
    },
    "model": {
      "type": "string",
      "enum": ["sd3-large", "sd3-large-turbo", "sd3-medium", "sdxl-1.0", "sd-1.6"],
      "default": "sd3-large"
    },
    "aspect_ratio": {
      "type": "string",
      "enum": ["1:1", "16:9", "9:16", "4:3", "3:4", "21:9", "9:21"],
      "default": "1:1"
    },
    "output_format": {
      "type": "string",
      "enum": ["png", "jpeg", "webp"],
      "default": "png"
    },
    "output_path": {
      "type": "string",
      "description": "Where to save the generated image"
    },
    "seed": {
      "type": "integer",
      "description": "Random seed for reproducibility"
    }
  },
  "required": ["prompt"]
}
```

### stability_img2img
Generate a new image based on a source image and prompt.
```json
{
  "type": "object",
  "properties": {
    "image_path": {
      "type": "string",
      "description": "Path to the source image"
    },
    "prompt": {
      "type": "string"
    },
    "negative_prompt": {
      "type": "string"
    },
    "strength": {
      "type": "number",
      "minimum": 0.0,
      "maximum": 1.0,
      "default": 0.7,
      "description": "How much to transform (0=keep original, 1=ignore original)"
    },
    "output_path": {
      "type": "string"
    }
  },
  "required": ["image_path", "prompt"]
}
```

### stability_upscale
Upscale an image to higher resolution.
```json
{
  "type": "object",
  "properties": {
    "image_path": {
      "type": "string",
      "description": "Path to the image to upscale"
    },
    "prompt": {
      "type": "string",
      "description": "Optional prompt to guide upscaling"
    },
    "output_format": {
      "type": "string",
      "enum": ["png", "jpeg", "webp"],
      "default": "png"
    },
    "output_path": {
      "type": "string"
    }
  },
  "required": ["image_path"]
}
```

### stability_inpaint
Edit a specific region of an image using a mask.
```json
{
  "type": "object",
  "properties": {
    "image_path": {
      "type": "string",
      "description": "Path to the source image"
    },
    "mask_path": {
      "type": "string",
      "description": "Path to the mask image (white = edit region)"
    },
    "prompt": {
      "type": "string",
      "description": "What to generate in the masked region"
    },
    "negative_prompt": {
      "type": "string"
    },
    "output_path": {
      "type": "string"
    }
  },
  "required": ["image_path", "mask_path", "prompt"]
}
```

### stability_remove_background
Remove the background from an image.
```json
{
  "type": "object",
  "properties": {
    "image_path": {
      "type": "string"
    },
    "output_path": {
      "type": "string"
    }
  },
  "required": ["image_path"]
}
```

## Commands

### generate
```bash
curl -s -X POST "https://api.stability.ai/v2beta/stable-image/generate/sd3" \
  -H "Authorization: Bearer $STABILITY_API_KEY" \
  -H "Accept: image/*" \
  -F "prompt={prompt}" \
  -F "model={model}" \
  -F "aspect_ratio={aspect_ratio}" \
  -F "output_format={output_format}" \
  -o "{output_path}"
```

### upscale
```bash
curl -s -X POST "https://api.stability.ai/v2beta/stable-image/upscale/conservative" \
  -H "Authorization: Bearer $STABILITY_API_KEY" \
  -H "Accept: image/*" \
  -F "image=@{image_path}" \
  -F "output_format={output_format}" \
  -o "{output_path}"
```

### remove_bg
```bash
curl -s -X POST "https://api.stability.ai/v2beta/stable-image/edit/remove-background" \
  -H "Authorization: Bearer $STABILITY_API_KEY" \
  -H "Accept: image/*" \
  -F "image=@{image_path}" \
  -o "{output_path}"
```

## Environment
- STABILITY_API_KEY

## Permissions
- network
- file_read
- file_write
