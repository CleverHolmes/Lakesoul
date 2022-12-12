package org.apache.spark.sql.lakesoul.kafka

import com.alibaba.fastjson.JSONObject
import com.dmetasoul.lakesoul.meta.DBManager
import org.apache.hadoop.fs.Path
import org.apache.spark.sql.functions.{col, from_json}
import org.apache.spark.sql.lakesoul.kafka.utils.KafkaUtils
import org.apache.spark.sql.lakesoul.utils.SparkUtil
import org.apache.spark.sql.types.{DataTypes, StructType}
import org.apache.spark.sql.{DataFrame, SparkSession}

import java.util
import java.util.UUID
import scala.collection.mutable

object KafkaStream {

  val brokers = "localhost:9092"
  val topic = "test"
  val warehouse = "/Users/dudongfeng/work/zehy/kafka"
  val namespace = "default"
  val KAFKA_TABLE_PREFIX = "kafka_table_"
  val dbManager = new DBManager()
  val checkpointPath = "/Users/dudongfeng/work/docker_compose/checkpoint/"

  def createTableIfNoExists(topicAndSchema: mutable.Map[String, StructType]): Unit = {
    topicAndSchema.foreach(info => {
      val tableName = info._1
      val schema = info._2.json
      val path = warehouse + "/" + namespace + "/" + tableName
      val tablePath = SparkUtil.makeQualifiedTablePath(new Path(path)).toString
      val tableExists = dbManager.isTableExistsByTableName(tableName, namespace)
      if (!tableExists) {
        val tableId = KAFKA_TABLE_PREFIX + UUID.randomUUID().toString
        dbManager.createNewTable(tableId, namespace, tableName, tablePath, schema, new JSONObject(), "")
      } else {
        val tableId = dbManager.shortTableName(tableName, namespace)
        dbManager.updateTableSchema(tableId.getTableId(), schema);
      }
    })
  }

  def createStreamDF(spark: SparkSession, brokers: String, topic: String, schema: StructType): DataFrame = {
    spark.readStream
      .format("kafka")
      .option("kafka.bootstrap.servers", brokers)
      .option("subscribe", topic)
      .option("startingOffsets", "earliest")
      .option("maxOffsetsPerTrigger", 100000)
      .option("enable.auto.commit", "false")
      .option("failOnDataLoss", false)
      .option("includeTimestamp", true)
      .load()
  }

  def topicValueToSchema(spark: SparkSession, topicAndMsg: util.Map[String, String]): mutable.Map[String, StructType] = {
    val map = new mutable.HashMap[String, StructType]
    topicAndMsg.keySet().forEach(topic => {
      var strList = List.empty[String]
      strList = strList :+ topicAndMsg.get(topic)
      val rddData = spark.sparkContext.parallelize(strList)
      val resultDF = spark.read.json(rddData)
      val schema = resultDF.schema

      var lakeSoulSchema = new StructType()
      schema.foreach( f => f.dataType match {
        case _: StructType => lakeSoulSchema = lakeSoulSchema.add(f.name, DataTypes.StringType, true)
        case _ => lakeSoulSchema = lakeSoulSchema.add(f.name, f.dataType, true)
      })
      map.put(topic, lakeSoulSchema)
    })
    map
  }



  def main(args: Array[String]): Unit = {

    val builder = SparkSession.builder()
      .appName("kafka-test")
      .master("local[4]")

    val spark = builder.getOrCreate()

    val topicAndMsg = KafkaUtils.getTopicMsg()
    val topicAndSchema = topicValueToSchema(spark, topicAndMsg)
    createTableIfNoExists(topicAndSchema)
    val topicString = topicAndMsg.keySet().toString.replace("[","").replace("]","").replace(" ","").strip()

    val multiTopicData = createStreamDF(spark, brokers, topicString, null)
      .selectExpr("CAST(value AS STRING) as value", "topic")
      .filter("value is not null")


    multiTopicData.writeStream.queryName("demo").foreachBatch {
      (batchDF: DataFrame, _: Long) => {
        for(topic <- topicAndSchema.keySet) {
          val path = warehouse + "/" + namespace + "/" + topic
          val topicDF = batchDF.filter(col("topic").equalTo(topic))
          if (!topicDF.rdd.isEmpty()) {
            topicDF.show(false)
            val rows = topicDF.withColumn("payload", from_json(col("value"), topicAndSchema.get(topic).get))
              .selectExpr("payload.*","topic")
            rows.show(false)
            rows.write.mode("append").format("lakesoul").option("mergeSchema", "true").save(path)
          }
        }
      }
    }
      .option("checkpointLocation", checkpointPath)
      .start().awaitTermination()
  }

}
