from sqlalchemy import Column, String, Text, DateTime, Integer, Float, ForeignKey, LargeBinary
from sqlalchemy.ext.declarative import declarative_base
from sqlalchemy.orm import relationship
from datetime import datetime

Base = declarative_base()


class Conversation(Base):
    __tablename__ = "conversations"

    id = Column(String, primary_key=True)
    title = Column(String, nullable=False)
    summary = Column(Text)
    created_at = Column(DateTime, nullable=False)
    updated_at = Column(DateTime, nullable=False)
    message_count = Column(Integer, default=0)
    topics = Column(Text)  # JSON array of keywords
    embedding = Column(LargeBinary)  # Vector for similarity search

    messages = relationship("Message", back_populates="conversation", cascade="all, delete-orphan")


class Message(Base):
    __tablename__ = "messages"

    id = Column(String, primary_key=True)
    conversation_id = Column(String, ForeignKey("conversations.id"), nullable=False)
    role = Column(String, nullable=False)  # 'human' or 'assistant'
    content = Column(Text, nullable=False)
    created_at = Column(DateTime, nullable=False)

    conversation = relationship("Conversation", back_populates="messages")


class Node(Base):
    __tablename__ = "nodes"

    id = Column(String, primary_key=True)
    conversation_id = Column(String, ForeignKey("conversations.id"))
    node_type = Column(String, nullable=False)  # 'conversation', 'topic', 'cluster'
    label = Column(String, nullable=False)
    position_x = Column(Float)
    position_y = Column(Float)
    color = Column(String)
    size = Column(Float)
    cluster_id = Column(Integer)


class Edge(Base):
    __tablename__ = "edges"

    id = Column(String, primary_key=True)
    source_id = Column(String, ForeignKey("nodes.id"), nullable=False)
    target_id = Column(String, ForeignKey("nodes.id"), nullable=False)
    relationship_type = Column(String, nullable=False)  # 'relates-to', 'similar'
    weight = Column(Float, default=1.0)
