"""Бенчмарк FastAPI: те же эндпоинты, Pydantic-валидация, asyncpg."""

import os

import asyncpg
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

app = FastAPI(openapi_url=None)
pool: asyncpg.Pool | None = None


@app.on_event("startup")
async def startup() -> None:
    global pool
    size = int(os.environ.get("DATABASE_POOL_SIZE_PER_WORKER", "4"))
    pool = await asyncpg.create_pool(os.environ["DATABASE_URL"], min_size=1, max_size=size)


@app.get("/")
async def hello():
    return {"message": "Hello, World!"}


class Item(BaseModel):
    name: str = Field(min_length=1, max_length=100)
    price: float = Field(gt=0)
    tags: list[str] = []


class ItemOut(BaseModel):
    id: int
    name: str
    price: float
    tags: list[str]


@app.post("/items")
async def create_item(item: Item) -> ItemOut:
    return ItemOut(id=1, name=item.name, price=item.price, tags=item.tags)


class Todo(BaseModel):
    id: int
    title: str
    done: bool


@app.get("/todos/{todo_id}")
async def read_todo(todo_id: int) -> Todo:
    row = await pool.fetchrow("SELECT id, title, done FROM bench_todos WHERE id = $1", todo_id)
    if row is None:
        raise HTTPException(404, "Todo not found")
    return Todo(**dict(row))
