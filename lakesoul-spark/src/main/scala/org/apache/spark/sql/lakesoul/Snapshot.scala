/*
 * Copyright [2022] [DMetaSoul Team]
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package org.apache.spark.sql.lakesoul

import com.dmetasoul.lakesoul.meta.{DataOperation, MetaUtils}
import org.apache.spark.sql.catalyst.expressions.Expression
import org.apache.spark.sql.execution.datasources.FileFormat
import org.apache.spark.sql.execution.datasources.parquet.ParquetFileFormat
import org.apache.spark.sql.lakesoul.utils._
import org.apache.spark.sql.{DataFrame, Dataset, SparkSession}


class Snapshot(table_info: TableInfo,
               partition_info_arr: Array[PartitionInfo],
               is_first_commit: Boolean = false
              ) {
  private var partitionDesc:String = ""
  private var startPartitionVersion:Int = -1
  private var endPartitionVersion:Int = -1
  private var incremental:Boolean = false
  def setPartitionDescAndVersion(parDesc:String,startParVer:Int,endParVer:Int,incremental:Boolean): Unit ={
    this.partitionDesc = parDesc
    this.startPartitionVersion = startParVer
    this.endPartitionVersion = endParVer
    this.incremental = incremental
  }
  def getPartitionDescAndVersion:(String,Int,Int,Boolean)={
    (this.partitionDesc,this.startPartitionVersion,this.endPartitionVersion,this.incremental)
  }
  def getTableName: String = table_info.table_path_s.get
  def getTableInfo: TableInfo = table_info
  def sizeInBytes(filters: Seq[Expression] = Nil): Long = {
    PartitionFilter.filesForScan(this, filters).map(_.size).sum
  }
  /** Return the underlying Spark `FileFormat` of the LakeSoulTableRel. */
  def fileFormat: FileFormat = new ParquetFileFormat()

  def getConfiguration: Map[String, String] = table_info.configuration

  def isFirstCommit: Boolean = is_first_commit

  def getPartitionInfoArray: Array[PartitionInfo] = partition_info_arr

}
