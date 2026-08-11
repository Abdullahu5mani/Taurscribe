#!/usr/bin/env python3
"""
Upload Parakeet Nemotron 0.6B FastConformer RNN-T MLX weights and card to Hugging Face.
"""
from pathlib import Path
import os
import sys
from huggingface_hub import HfApi

REPO_ID = "Abdullahu5mani/parakeet-nemotron-0.6b-mlx"
UPLOAD_DIR = Path("/tmp/parakeet_mlx_upload")

def main():
    token = os.environ.get("HF_TOKEN")
    api = HfApi(token=token)

    try:
        user_info = api.whoami()
        print(f"Authenticated as: {user_info.get('name')}")
    except Exception as e:
        print(f"Authentication check failed: {e}")
        sys.exit(1)

    if not UPLOAD_DIR.exists():
        print(f"Upload directory {UPLOAD_DIR} does not exist.")
        sys.exit(1)

    print(f"Files to upload from {UPLOAD_DIR}:")
    for f in sorted(UPLOAD_DIR.iterdir()):
        print(f"  - {f.name} ({f.stat().st_size:,} bytes)")

    print(f"\nEnsuring repository exists: https://huggingface.co/{REPO_ID}...")
    try:
        repo_url = api.create_repo(repo_id=REPO_ID, repo_type="model", exist_ok=True)
        print(f"Repository ready: {repo_url}")
    except Exception as e:
        print(f"Failed to create/access repo {REPO_ID}: {e}")
        print("\nNote: Make sure your HF token has 'Write' permissions or create the repository manually at:")
        print(f"  https://huggingface.co/new")
        sys.exit(1)

    print(f"Uploading files to {REPO_ID}...")
    api.upload_folder(
        folder_path=str(UPLOAD_DIR),
        repo_id=REPO_ID,
        repo_type="model",
        commit_message="Release Parakeet Nemotron 0.6B MLX weights, tokenizer, and unslop model card",
    )
    print(f"\nUpload complete: https://huggingface.co/{REPO_ID}")

if __name__ == "__main__":
    main()
