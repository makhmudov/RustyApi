"""Эталон OpenAPI для rustyapi/tests/openapi_parity.rs.

pip install fastapi email-validator && python3 fastapi_openapi.py fastapi_openapi.json
"""
import json
import sys
from enum import Enum
from typing import Optional

from fastapi import APIRouter, FastAPI, Form, Query
from pydantic import BaseModel, EmailStr, Field


class Status(str, Enum):
    todo = "todo"
    done = "done"


class Tag(BaseModel):
    name: str = Field(min_length=1)


class ItemCreate(BaseModel):
    name: str = Field(min_length=1, max_length=100)
    price: float = Field(gt=0)
    owner_email: EmailStr
    tags: list[str] = []
    description: Optional[str] = None
    code: Optional[str] = Field(default=None, pattern=r"^[A-Z]{3}$")
    status: Status = Status.todo
    labels: list[Tag] = Field(default=[], max_length=3)


class Item(BaseModel):
    id: int
    name: str
    price: float


app = FastAPI(title="Parity", version="1.2.3", description="Сверка с FastAPI")
items = APIRouter(prefix="/items", tags=["items"])


@items.get("/")
def list_items(q: str, limit: int = 10) -> list[Item]:
    return []


@items.get("/{item_id}")
def read_item(item_id: int) -> Item:
    """Читает товар по id."""
    return Item(id=item_id, name="x", price=1)


@items.post("/", status_code=201)
def create_item(item: ItemCreate) -> Item:
    return Item(id=1, name=item.name, price=item.price)


@items.delete("/{item_id}", status_code=204, deprecated=True)
def delete_item(item_id: int) -> None:
    return None


@app.get("/", summary="Say Hello", tags=["misc"])
def root() -> dict:
    return {}


@app.post("/login")
def login(username: str = Form(), password: str = Form()) -> str:
    return ""


app.include_router(items)
json.dump(app.openapi(), open(sys.argv[1], "w"), indent=1, ensure_ascii=False)
