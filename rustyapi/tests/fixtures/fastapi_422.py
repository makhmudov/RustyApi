from enum import Enum
from typing import Optional
from fastapi import FastAPI, Query, Form
from fastapi.testclient import TestClient
from pydantic import BaseModel, Field, EmailStr, HttpUrl
import json, sys

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
    homepage: Optional[HttpUrl] = None
    code: Optional[str] = Field(default=None, pattern=r"^[A-Z]{3}$")
    quantity: int = Field(default=1, ge=1, le=1000)
    status: Status = Status.todo
    labels: list[Tag] = Field(default=[], max_length=3)

app = FastAPI()
@app.post("/items")
def create(item: ItemCreate): return item
@app.get("/items/{item_id}")
def read(item_id: int): return {}
@app.get("/search")
def search(q: str, limit: int = 10): return {}
@app.post("/login")
def login(username: str = Form(), password: str = Form()): return {}

c = TestClient(app)
ok = {"name":"x","price":1,"owner_email":"a@example.com"}
cases = {
 "missing": ("post","/items",{"json":{"price":1,"owner_email":"a@example.com"}}),
 "short": ("post","/items",{"json":{**ok,"name":""}}),
 "long": ("post","/items",{"json":{**ok,"name":"a"*101}}),
 "gt": ("post","/items",{"json":{**ok,"price":0}}),
 "email": ("post","/items",{"json":{**ok,"owner_email":"nope"}}),
 "email2": ("post","/items",{"json":{**ok,"owner_email":"a@b"}}),
 "type_float": ("post","/items",{"json":{**ok,"price":"abc"}}),
 "type_float_bool": ("post","/items",{"json":{**ok,"price":True}}),
 "type_str": ("post","/items",{"json":{**ok,"name":5}}),
 "list_type": ("post","/items",{"json":{**ok,"tags":"a"}}),
 "list_item": ("post","/items",{"json":{**ok,"tags":["a",1]}}),
 "url": ("post","/items",{"json":{**ok,"homepage":"notaurl"}}),
 "url_scheme": ("post","/items",{"json":{**ok,"homepage":"ftp://x.com"}}),
 "pattern": ("post","/items",{"json":{**ok,"code":"ab"}}),
 "ge": ("post","/items",{"json":{**ok,"quantity":0}}),
 "le": ("post","/items",{"json":{**ok,"quantity":1001}}),
 "int_float": ("post","/items",{"json":{**ok,"quantity":1.5}}),
 "int_str": ("post","/items",{"json":{**ok,"quantity":"x"}}),
 "enum": ("post","/items",{"json":{**ok,"status":"zzz"}}),
 "nested": ("post","/items",{"json":{**ok,"labels":[{"name":""}]}}),
 "nested_missing": ("post","/items",{"json":{**ok,"labels":[{}]}}),
 "list_max": ("post","/items",{"json":{**ok,"labels":[{"name":"a"}]*4}}),
 "not_object": ("post","/items",{"json":[1]}),
 "empty_body": ("post","/items",{}),
 "bad_json": ("post","/items",{"content":b'{"name": ', "headers":{"content-type":"application/json"}}),
 "path_int": ("get","/items/abc",{}),
 "query_missing": ("get","/search",{}),
 "query_int": ("get","/search?q=a&limit=x",{}),
 "form_missing": ("post","/login",{"data":{"username":"u"}}),
}
out = {}
for k,(m,u,kw) in cases.items():
    r = getattr(c,m)(u,**kw)
    out[k] = {"status": r.status_code, "body": r.json()}
json.dump(out, open(sys.argv[1] if len(sys.argv) > 1 else "fastapi_422.json", "w"), indent=1, ensure_ascii=False)
for k,v in out.items(): print(k, json.dumps(v, ensure_ascii=False))
